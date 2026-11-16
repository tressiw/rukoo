// One Durable Object per account, holding everything about it in its own SQLite database: the account row, its
// sessions and its records. The Worker has already checked every input; this object checks secrets, keeps the
// sequence numbers and counters, and decides what gets stored.
//
// Methods are called over RPC and return { status, body } for the Worker to send. They first await every hash they
// need and then do all their reading and writing in one synchronous stretch, so no other call can slip in between a
// check and the write that depends on it.

import { DurableObject } from 'cloudflare:workers';
import { blob, randomBytes, sameBytes, sha256, toBase64url, utf8 } from './bytes.js';

// Ten failed sign-ins or recovery attempts within 15 minutes lock the account for 15 minutes (PROTOCOL.md, login).
// A session that sends a wrong password ten times is locked the same way, on its own (PROTOCOL.md, POST /v1/password).
const MAX_FAILURES = 10;
const FAILURE_WINDOW = 15 * 60 * 1000;
const LOCK_TIME = 15 * 60 * 1000;
// A device's last-seen time only needs to be roughly right, and writing it on every call would double the writes.
const SEEN_INTERVAL = 60 * 60 * 1000;
// Every sign-in adds a session. A device that keeps signing in without signing out would otherwise grow the list
// forever, so past this many the one seen longest ago goes.
const MAX_SESSIONS = 100;
// A pull stops early once it holds this much data and says there is more, so one page stays a size a phone can take.
const PAGE_BYTES = 4 * 1024 * 1024;
// An account can change its booking page's handle at most this often (PROTOCOL.md, Handles and meeting types).
export const HANDLE_CHANGES = 3;
export const HANDLE_WINDOW = 30 * 24 * 60 * 60 * 1000;

// A session's own count of wrong passwords and its lock, for the signed-in calls that ask for the password again.
const SESSION_LOCK = ['failures', 'failedAt', 'lockedUntil'].map((name) => `${name} INTEGER NOT NULL DEFAULT 0`);

// The booking page's handle, for accounts that ever had one. changes is a JSON list of when it was changed.
const BOOKING_TABLE = `
  CREATE TABLE IF NOT EXISTS booking (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    handle TEXT,
    changes TEXT NOT NULL DEFAULT '[]'
  );
`;

const SCHEMA = `
  CREATE TABLE account (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    email TEXT NOT NULL,
    createdAt INTEGER NOT NULL,
    kdf TEXT NOT NULL,
    salt BLOB NOT NULL,
    authVerifier BLOB NOT NULL,
    wrappedKey BLOB NOT NULL,
    recoveryVerifier BLOB NOT NULL,
    recoveryWrappedKey BLOB NOT NULL,
    failures INTEGER NOT NULL DEFAULT 0,
    failedAt INTEGER NOT NULL DEFAULT 0,
    lockedUntil INTEGER NOT NULL DEFAULT 0
  );
  CREATE TABLE sessions (
    id TEXT PRIMARY KEY,
    tokenHash BLOB NOT NULL,
    name TEXT NOT NULL,
    platform TEXT NOT NULL,
    createdAt INTEGER NOT NULL,
    lastSeenAt INTEGER NOT NULL,
    ${SESSION_LOCK.join(',\n    ')}
  );
  CREATE TABLE records (
    id TEXT PRIMARY KEY,
    seq INTEGER NOT NULL,
    data BLOB,
    size INTEGER NOT NULL,
    updatedAt INTEGER NOT NULL
  );
  CREATE INDEX records_seq ON records (seq);
  CREATE TABLE meta (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    seq INTEGER NOT NULL,
    records INTEGER NOT NULL,
    bytes INTEGER NOT NULL,
    tombstones INTEGER NOT NULL DEFAULT 0
  );
  ${BOOKING_TABLE}
`;

class QuotaExceeded extends Error {}

function reply(status, body = null, headers) {
  return { status, body, headers };
}

function failure(status, error, message, headers) {
  return reply(status, { error, message }, headers);
}

const UNAUTHORIZED = failure(401, 'unauthorized', 'Sign in again.');

function locked(until, now) {
  return failure(429, 'locked', 'Too many failed attempts. Try again later.', {
    'retry-after': String(Math.ceil((until - now) / 1000))
  });
}

export class Account extends DurableObject {
  constructor(ctx, env) {
    super(ctx, env);
    this.sql = ctx.storage.sql;
    // Tables are made at sign-up, not here: prelogin and failed sign-ins reach objects for addresses without an
    // account, and those objects should stay empty so they leave nothing behind.
    this.present =
      this.sql.exec("SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'account'").toArray().length > 0;
    if (this.present) this.migrate();
  }

  // Brings an account made by an older version of the server up to the current tables. Each step checks for itself
  // whether it is needed, so running it again changes nothing.
  migrate() {
    const columns = (table) =>
      new Set(this.sql.exec('SELECT name FROM pragma_table_info(?)', table).toArray().map((c) => c.name));
    if (!columns('meta').has('tombstones')) {
      // The first version didn't count tombstones; count the ones there are.
      this.ctx.storage.transactionSync(() => {
        this.sql.exec('ALTER TABLE meta ADD COLUMN tombstones INTEGER NOT NULL DEFAULT 0');
        this.sql.exec('UPDATE meta SET tombstones = (SELECT COUNT(*) FROM records WHERE data IS NULL) WHERE id = 1');
      });
    }
    if (!columns('sessions').has('lockedUntil')) {
      // Wrong passwords on signed-in calls counted toward the account's lock before; each session starts at none.
      this.ctx.storage.transactionSync(() => {
        for (const column of SESSION_LOCK) this.sql.exec(`ALTER TABLE sessions ADD COLUMN ${column}`);
      });
    }
    // Booking pages came later; accounts from before have no handle yet.
    this.sql.exec(BOOKING_TABLE);
  }

  // Whether this object holds an account. A Booking object asks once a day, so a page whose account was deleted
  // doesn't outlive it by more than a day. Asking leaves an empty object empty.
  async exists() {
    return this.present;
  }

  // Who is asking, for the booking page routes: the account's routing name (the token's first part, which is how the
  // Worker found this object), its handle, the caller's session id, how many handle changes the last 30 days used,
  // and the names of its sessions, for showing which device handles bookings. 401 only for a dead session.
  // passwordLock is the 429 to send while this session's password checks are locked (#withPassword), else null; only
  // DELETE /v1/booking uses it.
  async bookingAuth(token) {
    return this.#signedIn(token, (session, now) => {
      const row = this.#booking();
      const devices = {};
      for (const d of this.sql.exec('SELECT id, name FROM sessions')) devices[d.id] = d.name;
      return {
        status: 200,
        body: null,
        name: token.slice(0, token.indexOf('.')),
        handle: row.handle,
        deviceId: session.id,
        changes: recentChanges(row.changes, now).length,
        devices,
        passwordLock: session.lockedUntil > now ? locked(session.lockedUntil, now) : null
      };
    });
  }

  // Sets the account's one handle. Taking a first handle, or giving it up (null), isn't a change; replacing one is,
  // and the fourth within 30 days is refused.
  async bookingSetHandle(token, handle) {
    return this.#signedIn(token, (session, now) => {
      const row = this.#booking();
      const changes = recentChanges(row.changes, now);
      if (handle === row.handle) return reply(200, { handle, changes: changes.length });
      if (row.handle && handle) {
        if (changes.length >= HANDLE_CHANGES) {
          return failure(409, 'handle_changes', 'This account changed its handle 3 times in the last 30 days.');
        }
        changes.push(now);
      }
      this.sql.exec(
        'INSERT INTO booking (id, handle, changes) VALUES (1, ?, ?) ON CONFLICT (id) DO UPDATE SET handle = excluded.handle, changes = excluded.changes',
        handle,
        JSON.stringify(changes)
      );
      return reply(200, { handle, changes: changes.length });
    });
  }

  async prelogin() {
    if (!this.present) return reply(200, null);
    const { kdf, salt } = this.#account();
    return reply(200, { kdf: JSON.parse(kdf), salt: toBase64url(new Uint8Array(salt)) });
  }

  async signup(name, input, limits) {
    const authVerifier = await sha256(input.authKey);
    const recoveryVerifier = await sha256(input.recoveryAuth);
    const token = newToken(name);
    const tokenHash = await sha256(utf8(token));
    if (this.present) return failure(409, 'exists', 'This address already has an account.');
    const now = Date.now();
    let sessionId;
    this.ctx.storage.transactionSync(() => {
      this.sql.exec(SCHEMA);
      this.sql.exec(
        `INSERT INTO account (id, email, createdAt, kdf, salt, authVerifier, wrappedKey, recoveryVerifier, recoveryWrappedKey)
         VALUES (1, ?, ?, ?, ?, ?, ?, ?, ?)`,
        input.email,
        now,
        JSON.stringify(input.kdf),
        blob(input.salt),
        blob(authVerifier),
        blob(input.wrappedKey),
        blob(recoveryVerifier),
        blob(input.recoveryWrappedKey)
      );
      this.sql.exec('INSERT INTO meta (id, seq, records, bytes, tombstones) VALUES (1, 0, 0, 0, 0)');
      sessionId = this.#addSession(tokenHash, input.device, now);
    });
    this.present = true;
    return reply(201, { token, account: this.#info(sessionId, limits) });
  }

  async login(name, input, limits) {
    const hash = await sha256(input.authKey);
    const token = newToken(name);
    const tokenHash = await sha256(utf8(token));
    if (!this.present) return failure(401, 'unauthorized', 'The email address or password is wrong.');
    const now = Date.now();
    const check = this.#check('authVerifier', hash, now);
    if (check.result === 'locked') return locked(check.lockedUntil, now);
    if (check.result === 'wrong') return failure(401, 'unauthorized', 'The email address or password is wrong.');
    const sessionId = this.ctx.storage.transactionSync(() => this.#addSession(tokenHash, input.device, now));
    const { wrappedKey } = this.#account();
    return reply(200, {
      token,
      wrappedKey: toBase64url(new Uint8Array(wrappedKey)),
      account: this.#info(sessionId, limits)
    });
  }

  async logout(token) {
    return this.#signedIn(token, (session) => {
      this.sql.exec('DELETE FROM sessions WHERE id = ?', session.id);
      return reply(204);
    });
  }

  async account(token, limits) {
    return this.#signedIn(token, (session) => reply(200, this.#info(session.id, limits)));
  }

  async revoke(token, deviceId) {
    return this.#signedIn(token, () => {
      const { rowsWritten } = this.sql.exec('DELETE FROM sessions WHERE id = ?', deviceId);
      return rowsWritten > 0 ? reply(204) : failure(404, 'not_found', 'There is no device with this id.');
    });
  }

  async changePassword(token, input) {
    const hash = await sha256(input.authKey);
    const newVerifier = await sha256(input.newAuthKey);
    return this.#signedIn(token, (session, now) =>
      this.#withPassword(session, hash, now, () => {
        this.ctx.storage.transactionSync(() => {
          this.sql.exec(
            'UPDATE account SET kdf = ?, salt = ?, authVerifier = ?, wrappedKey = ? WHERE id = 1',
            JSON.stringify(input.kdf),
            blob(input.salt),
            blob(newVerifier),
            blob(input.wrappedKey)
          );
          this.sql.exec('DELETE FROM sessions WHERE id != ?', session.id);
        });
        return reply(204);
      })
    );
  }

  async replaceRecovery(token, input) {
    const hash = await sha256(input.authKey);
    const recoveryVerifier = await sha256(input.recoveryAuth);
    return this.#signedIn(token, (session, now) =>
      this.#withPassword(session, hash, now, () => {
        this.sql.exec(
          'UPDATE account SET recoveryVerifier = ?, recoveryWrappedKey = ? WHERE id = 1',
          blob(recoveryVerifier),
          blob(input.recoveryWrappedKey)
        );
        return reply(204);
      })
    );
  }

  // Checks the password and changes nothing but this session's count (PROTOCOL.md, POST /v1/password/check). A device
  // asks before it deletes what doesn't come back, such as the booking page, ahead of DELETE /v1/account: this counts
  // toward the same lock as that deletion, so the answer holds for it.
  async checkPassword(token, input) {
    const hash = await sha256(input.authKey);
    return this.#signedIn(token, (session, now) => this.#withPassword(session, hash, now, () => reply(204)));
  }

  async recoverStart(input) {
    const hash = await sha256(input.recoveryAuth);
    if (!this.present) return failure(401, 'unauthorized', 'The email address or recovery key is wrong.');
    const now = Date.now();
    const check = this.#check('recoveryVerifier', hash, now);
    if (check.result === 'locked') return locked(check.lockedUntil, now);
    if (check.result === 'wrong') return failure(401, 'unauthorized', 'The email address or recovery key is wrong.');
    const { recoveryWrappedKey, kdf } = this.#account();
    return reply(200, { recoveryWrappedKey: toBase64url(new Uint8Array(recoveryWrappedKey)), kdf: JSON.parse(kdf) });
  }

  async recoverFinish(name, input, limits) {
    const hash = await sha256(input.recoveryAuth);
    const authVerifier = await sha256(input.authKey);
    const token = newToken(name);
    const tokenHash = await sha256(utf8(token));
    if (!this.present) return failure(401, 'unauthorized', 'The email address or recovery key is wrong.');
    const now = Date.now();
    const check = this.#check('recoveryVerifier', hash, now);
    if (check.result === 'locked') return locked(check.lockedUntil, now);
    if (check.result === 'wrong') return failure(401, 'unauthorized', 'The email address or recovery key is wrong.');
    const sessionId = this.ctx.storage.transactionSync(() => {
      this.sql.exec(
        'UPDATE account SET kdf = ?, salt = ?, authVerifier = ?, wrappedKey = ? WHERE id = 1',
        JSON.stringify(input.kdf),
        blob(input.salt),
        blob(authVerifier),
        blob(input.wrappedKey)
      );
      this.sql.exec('DELETE FROM sessions');
      return this.#addSession(tokenHash, input.device, now);
    });
    return reply(200, { token, account: this.#info(sessionId, limits) });
  }

  async deleteAccount(token, input) {
    const hash = await sha256(input.authKey);
    // The account is gone from the moment the password checks out, before anything is awaited: a second delete
    // arriving meanwhile finds no account (401) instead of deleting it again, so the Worker gives its place under
    // MAX_ACCOUNTS back once. The booking handle is read in the same stretch and handed to the Worker, which wipes
    // the page afterwards (send() leaves the extra property out of the answer).
    const result = await this.#signedIn(token, (session, now) =>
      this.#withPassword(session, hash, now, () => {
        const { handle } = this.#booking();
        this.present = false;
        return { ...reply(204), handle };
      })
    );
    if (result.status !== 204) return result;
    try {
      // deleteAll removes the whole SQLite database, so the object is as empty as one that never had an account.
      await this.ctx.storage.deleteAll();
    } catch (err) {
      this.present =
        this.sql.exec("SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'account'").toArray().length > 0;
      throw err;
    }
    return result;
  }

  async pull(token, since, limit) {
    return this.#signedIn(token, () => {
      const rows = this.sql
        .exec('SELECT id, seq, data FROM records WHERE seq > ? ORDER BY seq LIMIT ?', since, limit + 1)
        .toArray();
      const records = [];
      let bytes = 0;
      for (const row of rows.slice(0, limit)) {
        if (records.length > 0 && bytes + (row.data?.byteLength ?? 0) > PAGE_BYTES) break;
        bytes += row.data?.byteLength ?? 0;
        records.push({
          id: row.id,
          seq: row.seq,
          data: row.data === null ? null : toBase64url(new Uint8Array(row.data))
        });
      }
      return reply(200, { records, seq: this.#meta().seq, more: records.length < rows.length });
    });
  }

  async push(token, items, limits) {
    return this.#signedIn(token, (session, now) => {
      try {
        return reply(
          200,
          this.ctx.storage.transactionSync(() => this.#store(items, limits, now))
        );
      } catch (err) {
        if (err instanceof QuotaExceeded) return failure(413, 'quota', 'This account is full.');
        throw err;
      }
    });
  }

  // Writes a push in one transaction. A delete keeps the row with its data cleared and a new seq, so devices that
  // pull later learn of it. Deleting what is already deleted, or was never stored, changes nothing and leaves no row.
  // The counters change in the same transaction, which keeps them exact without counting.
  #store(items, limits, now) {
    const start = this.#meta();
    let { seq, records, bytes, tombstones } = start;
    const results = [];
    for (const item of items) {
      const old = this.sql
        .exec('SELECT seq, size, data IS NOT NULL AS live FROM records WHERE id = ?', item.id)
        .toArray()[0];
      if (item.ifAbsent && old?.live) {
        results.push({ id: item.id, seq: old.seq, stored: false });
        continue;
      }
      if (!item.data && !old?.live) {
        results.push({ id: item.id, seq: old ? old.seq : seq, stored: true });
        continue;
      }
      seq += 1;
      if (old?.live) {
        records -= 1;
        bytes -= old.size;
      } else if (old) {
        tombstones -= 1;
      }
      const size = item.data ? item.data.byteLength : 0;
      if (item.data) {
        records += 1;
        bytes += size;
      } else {
        tombstones += 1;
      }
      this.sql.exec(
        `INSERT INTO records (id, seq, data, size, updatedAt) VALUES (?, ?, ?, ?, ?)
         ON CONFLICT (id) DO UPDATE SET seq = excluded.seq, data = excluded.data, size = excluded.size, updatedAt = excluded.updatedAt`,
        item.id,
        seq,
        item.data ? blob(item.data) : null,
        size,
        now
      );
      results.push({ id: item.id, seq, stored: true });
    }
    // Only a push that adds to an account over its limit is refused. One that was already over, for example after
    // the operator lowered a limit, can still delete and shrink records. Tombstones have a limit of their own, as many
    // as records: they take little room, but without a limit an account could grow them forever.
    const grows = (now, before, limit) => now > limit && now > before;
    if (
      grows(records, start.records, limits.records) ||
      grows(bytes, start.bytes, limits.bytes) ||
      grows(tombstones, start.tombstones, limits.records)
    ) {
      throw new QuotaExceeded();
    }
    this.sql.exec(
      'UPDATE meta SET seq = ?, records = ?, bytes = ?, tombstones = ? WHERE id = 1',
      seq,
      records,
      bytes,
      tombstones
    );
    return { results, seq };
  }

  // Runs fn with the session a token belongs to, or answers 401. Every session's hash is compared, without stopping
  // at a match, so the time taken says nothing about which tokens exist.
  async #signedIn(token, fn) {
    const hash = await sha256(utf8(token));
    if (!this.present) return UNAUTHORIZED;
    let session = null;
    for (const row of this.sql.exec('SELECT id, tokenHash, lastSeenAt, failures, failedAt, lockedUntil FROM sessions')) {
      if (sameBytes(new Uint8Array(row.tokenHash), hash)) session = row;
    }
    if (!session) return UNAUTHORIZED;
    const now = Date.now();
    if (now - session.lastSeenAt >= SEEN_INTERVAL) {
      this.sql.exec('UPDATE sessions SET lastSeenAt = ? WHERE id = ?', now, session.id);
    }
    return fn(session, now);
  }

  // For signed-in calls that ask for the password again. A wrong one is 403, not 401, so the app can tell it apart
  // from an ended session. These attempts count per session, not toward the account's lock: anyone who knows the
  // email address can lock that one, and the session already proves the device. A stolen session still gets only
  // ten guesses every 15 minutes, whatever other devices do: sign-ins leave its count and lock alone. While the lock
  // lasts, the session can't delete the booking page either (owner.js, deleteBooking).
  #withPassword(session, hash, now, fn) {
    const { authVerifier } = this.#account();
    const check = this.#attempt('sessions', session, sameBytes(new Uint8Array(authVerifier), hash), now);
    if (check.result === 'locked') return locked(check.lockedUntil, now);
    if (check.result === 'wrong') return failure(403, 'wrong_password', 'The password is wrong.');
    return fn();
  }

  // Compares a hash with the account's stored verifier and keeps the account's failure count, for sign-ins and
  // recovery.
  #check(column, hash, now) {
    const account = this.#account();
    return this.#attempt('account', account, sameBytes(new Uint8Array(account[column]), hash), now);
  }

  // Counts one attempt in row's failure count, where row is the account or a session from table and right says
  // whether the secret matched. Returns { result, lockedUntil }, where result is 'ok', 'wrong' or 'locked'. The
  // attempt that reaches the limit already answers 'locked', since the lock is in force from then on.
  #attempt(table, row, right, now) {
    const set = (failures, failedAt, lockedUntil) =>
      this.sql.exec(
        `UPDATE ${table} SET failures = ?, failedAt = ?, lockedUntil = ? WHERE id = ?`,
        failures,
        failedAt,
        lockedUntil,
        row.id
      );
    if (row.lockedUntil > now) return { result: 'locked', lockedUntil: row.lockedUntil };
    if (right) {
      if (row.failures > 0) set(0, 0, row.lockedUntil);
      return { result: 'ok' };
    }
    const fresh = now - row.failedAt >= FAILURE_WINDOW;
    const failures = fresh ? 1 : row.failures + 1;
    if (failures >= MAX_FAILURES) {
      set(0, 0, now + LOCK_TIME);
      return { result: 'locked', lockedUntil: now + LOCK_TIME };
    }
    set(failures, fresh ? now : row.failedAt, row.lockedUntil);
    return { result: 'wrong' };
  }

  #addSession(tokenHash, device, now) {
    const id = toBase64url(randomBytes(16));
    const count = this.sql.exec('SELECT COUNT(*) AS n FROM sessions').one().n;
    if (count >= MAX_SESSIONS) {
      this.sql.exec(
        'DELETE FROM sessions WHERE id IN (SELECT id FROM sessions ORDER BY lastSeenAt, createdAt LIMIT ?)',
        count - MAX_SESSIONS + 1
      );
    }
    this.sql.exec(
      'INSERT INTO sessions (id, tokenHash, name, platform, createdAt, lastSeenAt) VALUES (?, ?, ?, ?, ?, ?)',
      id,
      blob(tokenHash),
      device.name,
      device.platform,
      now,
      now
    );
    return id;
  }

  #account() {
    return this.sql.exec('SELECT * FROM account WHERE id = 1').one();
  }

  #meta() {
    return this.sql.exec('SELECT seq, records, bytes, tombstones FROM meta WHERE id = 1').one();
  }

  #booking() {
    return this.sql.exec('SELECT handle, changes FROM booking WHERE id = 1').toArray()[0] ?? { handle: null, changes: '[]' };
  }

  // What GET /v1/account returns, which sign-up, sign-in and recovery return too.
  #info(currentId, limits) {
    const account = this.#account();
    const meta = this.#meta();
    const devices = this.sql
      .exec('SELECT id, name, platform, createdAt, lastSeenAt FROM sessions ORDER BY createdAt, id')
      .toArray()
      .map((d) => ({ ...d, current: d.id === currentId }));
    return {
      email: account.email,
      createdAt: account.createdAt,
      kdf: JSON.parse(account.kdf),
      usage: { records: meta.records, bytes: meta.bytes },
      limits,
      devices
    };
  }
}

// A token names its account, so the Worker can route it without a table of sessions; the random part is the secret.
function newToken(name) {
  return `${name}.${toBase64url(randomBytes(32))}`;
}

// The handle changes within the last 30 days.
function recentChanges(json, now) {
  let list = [];
  try {
    list = JSON.parse(json);
  } catch {
    list = [];
  }
  return Array.isArray(list) ? list.filter((t) => Number.isFinite(t) && now - t < HANDLE_WINDOW) : [];
}
