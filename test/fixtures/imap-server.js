'use strict';

// An IMAP server for tests, over plain TCP on this PC. It greets, answers CAPABILITY and answers LOGIN with the
// response it was given, such as Gmail's refusal of a normal Google password. Given a mailbox, it also lists, opens and
// fetches from that one folder, enough for a sync. Any other command gets OK. It only reads single-line commands, which
// is all a LOGIN with a plain username and password takes.
const net = require('net');

const quote = (s) => `"${String(s).replace(/(["\\])/g, '\\$1')}"`;
const address = ([name, email]) => {
  const [mailbox, host] = email.split('@');
  return `((${quote(name)} NIL ${quote(mailbox)} ${quote(host)}))`;
};

// A single-part text/plain BODYSTRUCTURE of size bytes.
const textStructure = (size) => `("text" "plain" ("charset" "utf-8") NIL NIL "7bit" ${size} 1 NIL NIL NIL NIL)`;

// A BODYSTRUCTURE with a text part inside depth multiparts, each holding the next: the shape of a crafted message.
function nestedStructure(depth) {
  let structure = textStructure(5);
  for (let i = 0; i < depth; i++) structure = `(${structure} "mixed")`;
  return structure;
}

// The uids or sequence numbers a sequence set such as 1:*, 7,9 or 3:5 names, out of all.
function inSet(set, all) {
  const max = all.length ? Math.max(...all) : 0;
  const picked = new Set();
  for (const part of set.split(',')) {
    const [a, b = a] = part.split(':').map((n) => (n === '*' ? max : Number(n)));
    for (const n of all) if (n >= Math.min(a, b) && n <= Math.max(a, b)) picked.add(n);
  }
  return picked;
}

// One message's answer to a FETCH, with the items asked in the order asked.
function fetchItems(m, items) {
  const out = [];
  for (const item of items) {
    const upper = item.toUpperCase();
    if (upper === 'UID') out.push(`UID ${m.uid}`);
    else if (upper === 'FLAGS') out.push(`FLAGS (${(m.flags || []).join(' ')})`);
    else if (upper === 'INTERNALDATE') out.push(`INTERNALDATE ${quote(m.internalDate || '07-Nov-2026 10:00:00 +0000')}`);
    else if (upper === 'RFC822.SIZE') out.push(`RFC822.SIZE ${m.size || 100}`);
    else if (upper === 'BODYSTRUCTURE') out.push(`BODYSTRUCTURE ${m.bodyStructure}`);
    else if (upper === 'ENVELOPE') {
      const from = address(m.from || ['Sender', 'sender@example.com']);
      // m.date: the Date header as the server sends it, which may be text no parser reads; null for none (NIL).
      const date = m.date === null ? 'NIL' : quote(m.date || 'Sat, 07 Nov 2026 10:00:00 +0000');
      out.push(`ENVELOPE (${date} ${quote(m.subject || '')} ${from} ${from} ${from} ${address(['Me', 'me@example.com'])} NIL NIL NIL ${quote(`<${m.uid}@example.com>`)})`);
    } else {
      // BODY.PEEK[part]<0.max>: the start of a part, as a literal.
      const body = /^BODY\.PEEK\[([^\]]*)\](?:<(\d+)\.(\d+)>)?$/.exec(upper);
      if (!body) continue;
      const text = String((m.parts || {})[body[1]] || '').slice(0, body[3] ? Number(body[3]) : undefined);
      out.push(`BODY[${body[1]}]${body[2] ? `<${body[2]}>` : ''} {${Buffer.byteLength(text)}}\r\n${text}`);
    }
  }
  return out.join(' ');
}

// login: the status and text after the tag in the answer to LOGIN.
// mailbox: { path, uidValidity, messages: [{ uid, flags, subject, from: [name, email], date, internalDate, size,
// bodyStructure, parts }] }, the one folder there is. date is the envelope's Date header and internalDate the
// INTERNALDATE, both as text. bodyStructure is the IMAP text of the message's BODYSTRUCTURE; parts maps a part number
// to its text. Every command but LOGIN goes in commands, without its tag.
function imapServer({ login = 'OK LOGIN completed', mailbox = null } = {}) {
  const sockets = new Set();
  const logins = [];
  const commands = [];
  const server = net.createServer((socket) => {
    sockets.add(socket);
    socket.on('close', () => sockets.delete(socket));
    socket.on('error', () => {});
    socket.setEncoding('utf8');
    socket.write('* OK [CAPABILITY IMAP4rev1] Test server ready\r\n');
    let buffer = '';
    const fetch = (tag, byUid, set, list) => {
      const items = list.replace(/^\(|\)$/g, '').match(/BODY\.PEEK\[[^\]]*\](?:<[\d.]+>)?|[^\s()]+/gi) || [];
      const messages = mailbox.messages;
      const picked = inSet(set, byUid ? messages.map((m) => m.uid) : messages.map((_, i) => i + 1));
      messages.forEach((m, i) => {
        if (picked.has(byUid ? m.uid : i + 1)) socket.write(`* ${i + 1} FETCH (${fetchItems(m, byUid ? ['UID', ...items.filter((x) => x.toUpperCase() !== 'UID')] : items)})\r\n`);
      });
      socket.write(`${tag} OK ${byUid ? 'UID FETCH' : 'FETCH'} completed\r\n`);
    };
    socket.on('data', (chunk) => {
      buffer += chunk;
      let end;
      while ((end = buffer.indexOf('\r\n')) >= 0) {
        const line = buffer.slice(0, end);
        buffer = buffer.slice(end + 2);
        const [tag, command = ''] = line.split(' ');
        const rest = line.slice(tag.length + command.length + 2);
        if (command.toUpperCase() !== 'LOGIN') commands.push(line.slice(tag.length + 1));
        switch (command.toUpperCase()) {
          case 'CAPABILITY':
            socket.write(`* CAPABILITY IMAP4rev1\r\n${tag} OK CAPABILITY completed\r\n`);
            break;
          case 'LOGIN':
            logins.push(line);
            socket.write(`${tag} ${login}\r\n`);
            break;
          case 'LOGOUT':
            socket.end(`* BYE Logging out\r\n${tag} OK LOGOUT completed\r\n`);
            break;
          case 'LIST':
            if (mailbox) socket.write(`* LIST (\\HasNoChildren) "/" ${quote(mailbox.path || 'INBOX')}\r\n`);
            socket.write(`${tag} OK LIST completed\r\n`);
            break;
          case 'SELECT':
          case 'EXAMINE':
            if (!mailbox) {
              socket.write(`${tag} OK\r\n`);
              break;
            }
            socket.write(
              `* FLAGS (\\Answered \\Flagged \\Deleted \\Seen \\Draft)\r\n* ${mailbox.messages.length} EXISTS\r\n* OK [UIDVALIDITY ${mailbox.uidValidity || 1}] UIDs valid\r\n` +
                `* OK [UIDNEXT ${Math.max(0, ...mailbox.messages.map((m) => m.uid)) + 1}] Predicted next UID\r\n` +
                `${tag} OK [${command.toUpperCase() === 'EXAMINE' ? 'READ-ONLY' : 'READ-WRITE'}] ${command.toUpperCase()} completed\r\n`
            );
            break;
          case 'FETCH': {
            const [set, ...list] = rest.split(' ');
            if (mailbox) fetch(tag, false, set, list.join(' '));
            else socket.write(`${tag} OK\r\n`);
            break;
          }
          case 'UID': {
            const [sub, set, ...list] = rest.split(' ');
            if (mailbox && sub.toUpperCase() === 'FETCH') fetch(tag, true, set, list.join(' '));
            else socket.write(`${tag} OK\r\n`);
            break;
          }
          default:
            socket.write(`${tag} OK\r\n`);
        }
      }
    });
  });
  return new Promise((resolve, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', () =>
      resolve({
        port: server.address().port,
        logins,
        commands,
        close: () =>
          new Promise((done) => {
            for (const s of sockets) s.destroy();
            server.close(() => done());
          })
      })
    );
  });
}

// What Gmail answers to LOGIN with a normal Google password.
const GMAIL_APP_PASSWORD_REQUIRED = 'NO [ALERT] Application-specific password required: https://support.google.com/accounts/answer/185833 (Failure)';

// ---------- a mailbox with folders, for the app ----------

const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];
const pad2 = (n) => String(n).padStart(2, '0');
const imapDate = (ms) => {
  const d = new Date(ms);
  return `${pad2(d.getUTCDate())}-${MONTHS[d.getUTCMonth()]}-${d.getUTCFullYear()} ${pad2(d.getUTCHours())}:${pad2(d.getUTCMinutes())}:${pad2(d.getUTCSeconds())} +0000`;
};

// An IMAP argument string as its quoted strings, atoms (with any [section] kept whole) and parenthesised lists.
function tokens(s) {
  const out = [];
  let i = 0;
  while (i < s.length) {
    if (s[i] === ' ') {
      i++;
      continue;
    }
    let j = i;
    if (s[i] === '"') {
      let v = '';
      for (j = i + 1; j < s.length && s[j] !== '"'; j++) {
        if (s[j] === '\\') j++;
        v += s[j];
      }
      out.push(v);
      i = j + 1;
      continue;
    }
    if (s[i] === '(') {
      for (let depth = 0; j < s.length; j++) {
        if (s[j] === '(') depth++;
        else if (s[j] === ')' && --depth === 0) break;
      }
      out.push(s.slice(i, j + 1));
      i = j + 1;
      continue;
    }
    while (j < s.length && s[j] !== ' ') j = s[j] === '[' && s.indexOf(']', j) > 0 ? s.indexOf(']', j) + 1 : j + 1;
    out.push(s.slice(i, j));
    i = j;
  }
  return out;
}

// A mailbox the running app syncs against, on plain TCP on this PC: Inbox, Archive, Trash, Sent and Drafts, with
// enough of IMAP for a sync, opening an email, flags, moves, a search of unread mail and IDLE. put() files an email
// in a folder, also while the app runs, and the next sync finds it. An email: { subject, from: [name, address], date
// (ms, also its INTERNALDATE), dateHeader (the Date header as sent, which may be text no parser reads), flags, text }.
function imapMailbox({ email }) {
  const folders = new Map();
  for (const [p, use] of [['INBOX', null], ['Archive', '\\Archive'], ['Trash', '\\Trash'], ['Sent', '\\Sent'], ['Drafts', '\\Drafts']]) {
    folders.set(p, { path: p, specialUse: use, uidValidity: 7, nextUid: 1, messages: [] });
  }
  const sockets = new Set();
  const commands = [];
  const put = (folder, m) => {
    const f = folders.get(folder);
    const msg = {
      uid: f.nextUid++,
      flags: new Set(m.flags || []),
      subject: m.subject,
      from: m.from || ['Sender', 'sender@example.test'],
      date: m.date || Date.now(),
      dateHeader: m.dateHeader || new Date(m.date || Date.now()).toUTCString(),
      text: m.text || `Body of ${m.subject}`,
      messageId: m.messageId || `<${Date.now()}.${Math.random().toString(36).slice(2)}@fake.test>`
    };
    f.messages.push(msg);
    return msg;
  };
  const source = (m) => {
    const head = [`From: ${m.from[0]} <${m.from[1]}>`, `To: ${email}`, `Subject: ${m.subject}`, `Date: ${m.dateHeader}`, `Message-ID: ${m.messageId}`, 'MIME-Version: 1.0', 'Content-Type: text/plain; charset=utf-8', 'Content-Transfer-Encoding: 7bit'].join('\r\n');
    return { head: `${head}\r\n\r\n`, raw: `${head}\r\n\r\n${m.text}` };
  };
  const items = (m, list) => {
    const { head, raw } = source(m);
    const out = [];
    for (const item of list) {
      const upper = item.toUpperCase();
      if (upper === 'UID') out.push(`UID ${m.uid}`);
      else if (upper === 'FLAGS') out.push(`FLAGS (${[...m.flags].join(' ')})`);
      else if (upper === 'INTERNALDATE') out.push(`INTERNALDATE "${imapDate(m.date)}"`);
      else if (upper === 'RFC822.SIZE') out.push(`RFC822.SIZE ${Buffer.byteLength(raw)}`);
      else if (upper === 'BODYSTRUCTURE' || upper === 'BODY') out.push(`${upper} ${textStructure(Buffer.byteLength(m.text))}`);
      else if (upper === 'ENVELOPE') {
        const from = address(m.from);
        out.push(`ENVELOPE (${quote(m.dateHeader)} ${quote(m.subject)} ${from} ${from} ${from} ${address(['Me', email])} NIL NIL NIL ${quote(m.messageId)})`);
      } else {
        const body = /^BODY(?:\.PEEK)?\[([^\]]*)\](?:<(\d+)\.(\d+)>)?$/.exec(upper);
        if (!body) continue;
        const key = body[1];
        let text = key === '' ? raw : key === 'HEADER' ? head : key.startsWith('HEADER.FIELDS') ? '\r\n' : key === 'TEXT' || key === '1' ? m.text : '';
        if (body[3]) text = text.slice(Number(body[2]), Number(body[2]) + Number(body[3]));
        out.push(`BODY[${key}]${body[2] ? `<${body[2]}>` : ''} {${Buffer.byteLength(text)}}\r\n${text}`);
      }
    }
    return out.join(' ');
  };
  const server = net.createServer((socket) => {
    sockets.add(socket);
    socket.on('close', () => sockets.delete(socket));
    socket.on('error', () => {});
    socket.setEncoding('utf8');
    socket.write('* OK [CAPABILITY IMAP4rev1 MOVE UIDPLUS] Test mailbox ready\r\n');
    let buffer = '';
    let selected = null;
    let idleTag = null;
    const w = (s) => socket.write(s);
    const pick = (byUid, set) => {
      const msgs = selected ? selected.messages : [];
      const ids = inSet(set, byUid ? msgs.map((m) => m.uid) : msgs.map((_, i) => i + 1));
      return msgs.filter((m, i) => ids.has(byUid ? m.uid : i + 1));
    };
    const handle = (line) => {
      if (idleTag) {
        if (line.trim().toUpperCase() === 'DONE') {
          w(`${idleTag} OK IDLE terminated\r\n`);
          idleTag = null;
        }
        return;
      }
      const sp = line.indexOf(' ');
      if (sp < 0) return;
      const tag = line.slice(0, sp);
      const all = line.slice(sp + 1);
      let [cmd, ...rest] = all.split(' ');
      let args = rest.join(' ');
      const byUid = cmd.toUpperCase() === 'UID';
      if (byUid) {
        [cmd, ...rest] = args.split(' ');
        args = rest.join(' ');
      }
      const C = cmd.toUpperCase();
      if (C !== 'LOGIN') commands.push(all);
      switch (C) {
        case 'CAPABILITY':
          return w(`* CAPABILITY IMAP4rev1 MOVE UIDPLUS\r\n${tag} OK CAPABILITY completed\r\n`);
        case 'LOGOUT':
          return socket.end(`* BYE Logging out\r\n${tag} OK LOGOUT completed\r\n`);
        case 'LIST':
        case 'LSUB': {
          const a = tokens(args);
          if (a.length >= 2 && a[a.length - 1] === '') w(`* ${C} (\\Noselect) "/" ""\r\n`);
          else for (const f of folders.values()) w(`* ${C} (\\HasNoChildren${f.specialUse ? ` ${f.specialUse}` : ''}) "/" ${quote(f.path)}\r\n`);
          return w(`${tag} OK ${C} completed\r\n`);
        }
        case 'SELECT':
        case 'EXAMINE': {
          const name = tokens(args)[0];
          const f = folders.get(name) || (String(name).toUpperCase() === 'INBOX' ? folders.get('INBOX') : null);
          if (!f) return w(`${tag} NO No such mailbox\r\n`);
          selected = f;
          return w(
            `* FLAGS (\\Answered \\Flagged \\Deleted \\Seen \\Draft)\r\n* OK [PERMANENTFLAGS (\\Answered \\Flagged \\Deleted \\Seen \\Draft \\*)] Flags permitted\r\n` +
              `* ${f.messages.length} EXISTS\r\n* 0 RECENT\r\n* OK [UIDVALIDITY ${f.uidValidity}] UIDs valid\r\n* OK [UIDNEXT ${f.nextUid}] Predicted next UID\r\n` +
              `${tag} OK [${C === 'EXAMINE' ? 'READ-ONLY' : 'READ-WRITE'}] ${C} completed\r\n`
          );
        }
        case 'STATUS': {
          const f = folders.get(tokens(args)[0]);
          if (!f) return w(`${tag} NO No such mailbox\r\n`);
          const unseen = f.messages.filter((m) => !m.flags.has('\\Seen')).length;
          return w(`* STATUS ${quote(f.path)} (MESSAGES ${f.messages.length} UIDNEXT ${f.nextUid} UIDVALIDITY ${f.uidValidity} UNSEEN ${unseen} RECENT 0)\r\n${tag} OK STATUS completed\r\n`);
        }
        case 'FETCH': {
          const at = args.indexOf(' ');
          const asked = args.slice(at + 1).replace(/^\(|\)$/g, '').match(/BODY(?:\.PEEK)?\[[^\]]*\](?:<[\d.]+>)?|[^\s()]+/gi) || [];
          for (const m of pick(byUid, args.slice(0, at))) {
            w(`* ${selected.messages.indexOf(m) + 1} FETCH (${items(m, byUid ? ['UID', ...asked.filter((x) => x.toUpperCase() !== 'UID')] : asked)})\r\n`);
          }
          return w(`${tag} OK ${byUid ? 'UID FETCH' : 'FETCH'} completed\r\n`);
        }
        case 'STORE': {
          const s = /^(\S+)\s+([+-]?)FLAGS(\.SILENT)?\s+\(?([^)]*)\)?/i.exec(args);
          if (!s) return w(`${tag} BAD\r\n`);
          const flags = s[4].split(/\s+/).filter(Boolean);
          for (const m of pick(byUid, s[1])) {
            if (s[2] === '+') flags.forEach((f) => m.flags.add(f));
            else if (s[2] === '-') flags.forEach((f) => m.flags.delete(f));
            else m.flags = new Set(flags);
            if (!s[3]) w(`* ${selected.messages.indexOf(m) + 1} FETCH (UID ${m.uid} FLAGS (${[...m.flags].join(' ')}))\r\n`);
          }
          return w(`${tag} OK STORE completed\r\n`);
        }
        case 'MOVE':
        case 'COPY': {
          const [set, destName] = tokens(args);
          const dest = folders.get(destName);
          if (!dest || !selected) return w(`${tag} NO [TRYCREATE] No such mailbox\r\n`);
          const msgs = pick(byUid, set);
          const src = [];
          const dst = [];
          for (const m of msgs) {
            const copy = { ...m, flags: new Set(m.flags), uid: dest.nextUid++ };
            dest.messages.push(copy);
            src.push(m.uid);
            dst.push(copy.uid);
          }
          const code = `COPYUID ${dest.uidValidity} ${src.join(',')} ${dst.join(',')}`;
          if (C === 'COPY') return w(`${tag} OK [${code}] COPY completed\r\n`);
          w(`* OK [${code}] Moved\r\n`);
          for (const seq of msgs.map((m) => selected.messages.indexOf(m) + 1).sort((a, b) => b - a)) {
            selected.messages.splice(seq - 1, 1);
            w(`* ${seq} EXPUNGE\r\n`);
          }
          return w(`${tag} OK MOVE completed\r\n`);
        }
        case 'EXPUNGE': {
          if (!selected) return w(`${tag} OK\r\n`);
          for (let i = selected.messages.length - 1; i >= 0; i--) {
            if (selected.messages[i].flags.has('\\Deleted')) {
              selected.messages.splice(i, 1);
              w(`* ${i + 1} EXPUNGE\r\n`);
            }
          }
          return w(`${tag} OK EXPUNGE completed\r\n`);
        }
        case 'SEARCH': {
          // Unread or read mail, within a uid range; anything else finds nothing.
          const a = tokens(args).map((x) => x.toUpperCase());
          let msgs = selected ? [...selected.messages] : [];
          for (let i = 0; i < a.length; i++) {
            if (a[i] === 'UNSEEN' || (a[i] === 'NOT' && a[i + 1] === 'SEEN' && ++i)) msgs = msgs.filter((m) => !m.flags.has('\\Seen'));
            else if (a[i] === 'SEEN') msgs = msgs.filter((m) => m.flags.has('\\Seen'));
            else if (a[i] === 'UID') {
              const ids = inSet(a[++i], selected.messages.map((m) => m.uid));
              msgs = msgs.filter((m) => ids.has(m.uid));
            } else if (!['ALL', 'CHARSET', 'UTF-8'].includes(a[i]) && !a[i].startsWith('RETURN')) msgs = [];
          }
          const ids = msgs.map((m) => (byUid ? m.uid : selected.messages.indexOf(m) + 1));
          return w(`* SEARCH${ids.length ? ` ${ids.join(' ')}` : ''}\r\n${tag} OK SEARCH completed\r\n`);
        }
        case 'IDLE':
          idleTag = tag;
          return w('+ idling\r\n');
        case 'CLOSE':
        case 'UNSELECT':
          selected = null;
          return w(`${tag} OK ${C} completed\r\n`);
        default:
          return w(`${tag} OK ${C} completed\r\n`);
      }
    };
    socket.on('data', (chunk) => {
      buffer += chunk;
      let end;
      while ((end = buffer.indexOf('\r\n')) >= 0) {
        const line = buffer.slice(0, end);
        buffer = buffer.slice(end + 2);
        try {
          handle(line);
        } catch (err) {
          commands.push(`!! ${err.message}`);
        }
      }
    });
  });
  return new Promise((resolve, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', () =>
      resolve({
        port: server.address().port,
        email,
        folders,
        commands,
        put,
        close: () =>
          new Promise((done) => {
            for (const s of sockets) s.destroy();
            server.close(() => done());
          })
      })
    );
  });
}

module.exports = { imapServer, imapMailbox, nestedStructure, textStructure, GMAIL_APP_PASSWORD_REQUIRED };
