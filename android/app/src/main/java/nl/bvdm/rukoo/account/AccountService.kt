package nl.bvdm.rukoo.account

import java.io.File
import java.time.Instant
import java.time.ZoneOffset
import java.time.format.DateTimeFormatter
import java.util.Base64
import java.util.concurrent.atomic.AtomicLong
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Deferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.async
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.add
import kotlinx.serialization.json.addJsonObject
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.buildJsonArray
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import kotlinx.serialization.json.putJsonArray
import kotlinx.serialization.json.putJsonObject
import nl.bvdm.rukoo.core.AppLog
import nl.bvdm.rukoo.core.Clock
import nl.bvdm.rukoo.core.JsonFiles
import nl.bvdm.rukoo.core.RukooJson
import nl.bvdm.rukoo.core.SecretCipher
import nl.bvdm.rukoo.core.UriComponent

/**
 * A Rukoo account: Rukoo's own data (categories, the category each email got, corrections) on every device of one
 * person, end-to-end encrypted. A port of desktop's src/main/account/index.js; docs/account.md says what it does and
 * server/PROTOCOL.md how.
 *
 * The data lives on the phone first. Records work without an account and stay here; signing in sends them to the
 * account and brings in what the other devices wrote. The server is never required: when it can't be reached,
 * everything keeps working here and goes out at the next sync.
 *
 * Desktop runs this on one thread; here the state sits behind one lock, taken for each stretch between two network
 * calls, so the same interleavings are possible as with desktop's awaits and no others.
 */
class AccountService(
    private val dir: File,
    private val secrets: SecretCipher,
    defaultServer: String,
    deviceName: String,
    private val http: AccountHttp,
    scope: CoroutineScope,
    /** Whether plain http to this machine and the emulator's host is allowed: debug builds and tests. */
    private val devHosts: Boolean = false,
    private val platform: String = "android",
    private val clock: Clock = Clock.System,
    private val pushAfterMs: Long = PUSH_AFTER,
    private val saveAfterMs: Long = SAVE_AFTER,
) {
    private val lock = Any()

    // Its own supervisor, so a failed sync never cancels the scope it was given, and off the main thread: a sync
    // writes files and seals records.
    private val scope = CoroutineScope(scope.coroutineContext + SupervisorJob(scope.coroutineContext[Job]) + Dispatchers.Default)
    val defaultServer: String = AccountServers.serverUrl(defaultServer, devHosts).orEmpty()
    private val deviceName = deviceName.ifBlank { "Rukoo" }.take(100)

    /** One sign-in: what it was made with, compared by identity, so answers for an old one can be told apart. */
    class Session(val token: String, val dataKey: ByteArray, val keys: RecordKeys)

    private class Syncing(val session: Session, val job: Deferred<AccountStatus>)

    private var state = AccountState()

    // The cursor as of the records last written to disk: the state on disk never goes past it.
    private var durableCursor = 0L
    private val entries = LinkedHashMap<String, AccountEntry>()
    private val ids = HashMap<String, String>()
    private var session: Session? = null
    private var syncing: Syncing? = null
    private var pushJob: Job? = null
    private var saveJob: Job? = null

    private val _status = MutableStateFlow(AccountStatus())
    val status: StateFlow<AccountStatus> = _status.asStateFlow()

    private val _records = MutableSharedFlow<Set<String>>(extraBufferCapacity = 64)

    /** The collections whose records changed, locally or from another device (desktop's 'records' event). */
    val records: SharedFlow<Set<String>> = _records

    private val _version = MutableStateFlow(0L)

    /** Goes up whenever any record changes, for screens that show records. */
    val version: StateFlow<Long> = _version.asStateFlow()

    private val stateFile get() = File(dir, "account.json")
    private val recordsFile get() = File(dir, "account-records.json")

    init {
        load()
    }

    private fun load() = synchronized(lock) {
        state = runCatching { JsonFiles.read(stateFile, AccountState.serializer(), AccountState()) }.getOrElse { AccountState() }
        val saved = runCatching { JsonFiles.read(recordsFile, StoredEntries.serializer(), StoredEntries()) }.getOrElse { StoredEntries() }
        for (e in saved.entries) entries[entryKey(e.c, e.k)] = e
        durableCursor = state.cursor
        val email = state.email
        val token = state.token
        val dataKey = state.dataKey
        if (email != null && token != null && dataKey != null) {
            pin()
            val plainToken = secrets.decrypt(token)
            val key = secrets.decrypt(dataKey)?.let { runCatching { Base64.getDecoder().decode(it) }.getOrNull() }
            if (plainToken != null && key != null && key.size == 32) {
                open(plainToken, key)
            } else {
                // Stored under a Keystore key this phone no longer has: the keys don't open here, so it signs in again.
                state = state.copy(error = "signed_out")
                forget()
            }
        }
        _status.value = statusNow()
    }

    val server: String get() = synchronized(lock) { state.server ?: defaultServer }

    /**
     * A session belongs to the address it signed in at, because the password proofs include it (server/PROTOCOL.md,
     * Keys). A phone that signed in at the default keeps that address when a newer Rukoo has another default. Sessions
     * from before [AccountState.sessionServer] was kept have the server info fetched at sign-in, which is for the same
     * address. Under the lock.
     */
    private fun pin() {
        if (state.server != null) return
        val at = AccountServers.serverUrl(state.sessionServer ?: state.info?.server, devHosts) ?: return
        if (at == defaultServer) return
        state = state.copy(server = at)
        saveState()
    }

    val signedIn: Boolean get() = synchronized(lock) { session != null }

    fun status(): AccountStatus = synchronized(lock) { statusNow() }

    private fun statusNow(): AccountStatus {
        val counts = HashMap<String, Int>()
        var pending = 0
        for (e in entries.values) {
            if (e.dirty) pending++
            if (!e.deleted) counts[e.c] = (counts[e.c] ?: 0) + 1
        }
        val server = state.server ?: defaultServer
        val signedIn = session != null
        return AccountStatus(
            server = server,
            defaultServer = defaultServer,
            signedIn = signedIn,
            email = if (signedIn) state.email else null,
            lastSync = state.lastSync,
            syncing = syncing != null && syncing?.session === session,
            error = state.error?.let(AccountError::known),
            info = state.info?.takeIf { it.server == server }?.data,
            pending = if (signedIn) pending else 0,
            counts = counts,
        )
    }

    // ---- talking to the server ----

    private fun fail(code: String, status: Int = 0) = AccountError(AccountError.known(code), status)

    private fun translated(e: Throwable): Throwable = if (e is AccountError) fail(e.code, e.status) else e

    /**
     * [session]: the one a signed-in call is made for. Its answer may come after this phone signed out or into
     * another account; a 401 then says nothing about the session there is now. [meter]: what the answer's size is
     * added to.
     */
    private suspend fun call(
        method: String,
        route: String,
        body: JsonElement? = null,
        auth: Boolean = false,
        server: String = this.server,
        session: Session? = synchronized(lock) { this.session },
        meter: AtomicLong? = null,
    ): JsonElement? {
        if (server.isEmpty()) throw fail("no_server")
        try {
            return http.request(server, method, route, body, if (auth) session?.token else null, meter)
        } catch (e: AccountError) {
            // The session ended elsewhere: this phone was removed or the password changed on another device.
            if (auth && e.status == 401) {
                val ended = synchronized(lock) {
                    if (session != null && this.session === session) {
                        state = state.copy(error = "signed_out")
                        forget()
                        true
                    } else {
                        false
                    }
                }
                if (ended) changedStatus()
                throw fail("signed_out", 401)
            }
            throw translated(e)
        }
    }

    /** Who runs the server and where it keeps data; shown before anyone signs up. The current server's is kept. */
    suspend fun serverInfo(url: String = server): ServerInfo {
        val target = AccountServers.serverUrl(url, devHosts) ?: throw fail("no_server")
        val data = call("GET", "/v1/info", server = target) as? JsonObject
        val protocol = (data?.get("protocol") as? JsonPrimitive)?.takeIf { !it.isString }?.content?.toDoubleOrNull()
        if (data == null || protocol != PROTOCOL.toDouble()) throw fail("protocol")
        val info = runCatching { RukooJson.decodeFromJsonElement(ServerInfo.serializer(), data) }.getOrElse { throw fail("protocol") }
        val kept = synchronized(lock) {
            if (target == (state.server ?: defaultServer)) {
                state = state.copy(info = StoredInfo(target, info, clock.now()))
                saveState()
                true
            } else {
                false
            }
        }
        if (kept) changedStatus()
        return info
    }

    fun setServer(url: String): AccountStatus {
        synchronized(lock) {
            if (session != null) throw fail("signed_in")
            val next = if (url.isNotEmpty()) AccountServers.serverUrl(url, devHosts) ?: throw fail("no_server") else ""
            state = state.copy(server = next.takeIf { it.isNotEmpty() && it != defaultServer }, info = null)
            saveState()
        }
        changedStatus()
        return status()
    }

    private fun device(): JsonObject = buildJsonObject {
        put("name", deviceName)
        put("platform", platform)
    }

    // ---- signing up, in and out ----

    private fun checkNew(email: String, password: String) {
        if (signedIn) throw fail("signed_in")
        if (!validEmail(email)) throw fail("invalid_email")
        if (password.codePointCount(0, password.length) < MIN_PASSWORD) throw fail("weak_password")
        checkStorage()
    }

    /** The session has to be stored safely (the Keystore box); without one, stop before the server makes an account. */
    private fun checkStorage() {
        secrets.encrypt("rukoo")
    }

    /** The password's keys for the server this phone uses; its address is part of authKey (server/PROTOCOL.md, Keys). */
    private suspend fun passwordKeys(password: String, salt: ByteArray, kdf: Kdf) =
        withContext(Dispatchers.Default) {
            try {
                AccountKeys.passwordKeys(password, salt, kdf, server)
            } catch (e: OutOfMemoryError) {
                // The hash's memory is one block that is freed again; the app carries on.
                throw fail("heavy_kdf")
            }
        }

    private fun recoveryKeys(key: ByteArray) = AccountKeys.recoveryKeys(key, server)

    suspend fun signUp(email: String, password: String, invite: String = ""): SignUpResult {
        val address = AccountKeys.normalizeEmail(email)
        checkNew(address, password)
        serverInfo()
        val salt = AccountKeys.randomBytes(16)
        val kdf = AccountKeys.DEFAULT_KDF
        val keys = passwordKeys(password, salt, kdf)
        val dataKey = AccountKeys.randomBytes(32)
        val recoveryKey = AccountKeys.randomBytes(20)
        val recovery = recoveryKeys(recoveryKey)
        val res = call(
            "POST",
            "/v1/signup",
            buildJsonObject {
                put("email", address)
                if (invite.isNotEmpty()) put("invite", invite.trim())
                put("kdf", kdf.toJson())
                put("salt", AccountKeys.b64u(salt))
                put("authKey", AccountKeys.b64u(keys.authKey))
                put("wrappedKey", AccountKeys.b64u(AccountKeys.wrapDataKey(keys.wrapKey, dataKey)))
                put("recoveryAuth", AccountKeys.b64u(recovery.recoveryAuth))
                put("recoveryWrappedKey", AccountKeys.b64u(AccountKeys.wrapDataKey(recovery.recoveryWrap, dataKey)))
                put("device", device())
            },
        ) as? JsonObject
        attach(address, tokenOf(res), dataKey)
        return SignUpResult(AccountKeys.formatRecoveryKey(recoveryKey), detailsOf(res?.get("account")))
    }

    private fun tokenOf(res: JsonObject?): String =
        (res?.get("token") as? JsonPrimitive)?.takeIf { it.isString && it.content.isNotEmpty() }?.content ?: throw fail("server")

    private fun detailsOf(e: JsonElement?): AccountDetails? = (e as? JsonObject)?.let { runCatching { RukooJson.decodeFromJsonElement(AccountDetails.serializer(), it) }.getOrNull() }

    /** authKey for the password, with the parameters the server keeps for the address. Refuses weak ones. */
    private suspend fun authFor(email: String, password: String): AccountKeys.PasswordKeys {
        val pre = call("POST", "/v1/prelogin", buildJsonObject { put("email", email) }) as? JsonObject
        val kdf = try {
            AccountKeys.checkKdf(pre?.get("kdf"))
        } catch (e: IllegalArgumentException) {
            throw fail("weak_kdf")
        }
        if (!AccountKeys.affordable(kdf)) throw fail("heavy_kdf")
        val salt = try {
            AccountKeys.unb64u((pre?.get("salt") as? JsonPrimitive)?.takeIf { it.isString }?.content, 16)
        } catch (e: IllegalArgumentException) {
            throw fail("server")
        }
        return passwordKeys(password, salt, kdf)
    }

    suspend fun signIn(email: String, password: String): AccountDetails? {
        val address = AccountKeys.normalizeEmail(email)
        if (signedIn) throw fail("signed_in")
        if (!validEmail(address)) throw fail("invalid_email")
        if (password.isEmpty()) throw fail("unauthorized")
        checkStorage()
        serverInfo()
        val keys = authFor(address, password)
        val res = call("POST", "/v1/login", buildJsonObject {
            put("email", address)
            put("authKey", AccountKeys.b64u(keys.authKey))
            put("device", device())
        }) as? JsonObject
        val dataKey = try {
            AccountKeys.unwrapDataKey(keys.wrapKey, AccountKeys.unb64u((res?.get("wrappedKey") as? JsonPrimitive)?.content))
        } catch (e: Exception) {
            throw fail("server")
        }
        attach(address, tokenOf(res), dataKey)
        return detailsOf(res?.get("account"))
    }

    /** A forgotten password: the recovery key opens the data key, and a new password takes over. */
    suspend fun recover(email: String, recoveryKey: String, password: String): AccountDetails? {
        val address = AccountKeys.normalizeEmail(email)
        checkNew(address, password)
        val key = AccountKeys.parseRecoveryKey(recoveryKey) ?: throw fail("recovery_key")
        serverInfo()
        val recovery = recoveryKeys(key)
        val start = call("POST", "/v1/recover/start", buildJsonObject {
            put("email", address)
            put("recoveryAuth", AccountKeys.b64u(recovery.recoveryAuth))
        }) as? JsonObject
        val dataKey = try {
            AccountKeys.unwrapDataKey(recovery.recoveryWrap, AccountKeys.unb64u((start?.get("recoveryWrappedKey") as? JsonPrimitive)?.content))
        } catch (e: Exception) {
            throw fail("server")
        }
        val salt = AccountKeys.randomBytes(16)
        val kdf = AccountKeys.DEFAULT_KDF
        val keys = passwordKeys(password, salt, kdf)
        val res = call("POST", "/v1/recover/finish", buildJsonObject {
            put("email", address)
            put("recoveryAuth", AccountKeys.b64u(recovery.recoveryAuth))
            put("kdf", kdf.toJson())
            put("salt", AccountKeys.b64u(salt))
            put("authKey", AccountKeys.b64u(keys.authKey))
            put("wrappedKey", AccountKeys.b64u(AccountKeys.wrapDataKey(keys.wrapKey, dataKey)))
            put("device", device())
        }) as? JsonObject
        attach(address, tokenOf(res), dataKey)
        return detailsOf(res?.get("account"))
    }

    /** Only the session this was asked for: a slow answer must not sign out one made meanwhile. */
    suspend fun signOut(): AccountStatus {
        val session = synchronized(lock) { session } ?: return status()
        try {
            call("POST", "/v1/logout", auth = true, session = session)
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            // Signed out here all the same; the server ends the session when it next hears of it.
        }
        if (endIf(session)) changedStatus()
        return status()
    }

    /** Signs this phone out when [session] is still the one it has; true when it did. */
    private fun endIf(session: Session): Boolean = synchronized(lock) {
        if (this.session !== session) return false
        state = state.copy(error = null)
        forget()
        true
    }

    suspend fun details(): AccountDetails {
        if (!signedIn) throw fail("signed_out")
        return detailsOf(call("GET", "/v1/account", auth = true)) ?: throw fail("server")
    }

    suspend fun removeDevice(id: String): Boolean {
        if (!signedIn) throw fail("signed_out")
        call("DELETE", "/v1/devices/${UriComponent.encode(id)}", auth = true)
        return true
    }

    private fun email(): String = synchronized(lock) { state.email.orEmpty() }

    private fun dataKey(): ByteArray = synchronized(lock) { session?.dataKey } ?: throw fail("signed_out")

    suspend fun changePassword(current: String, next: String): Boolean {
        if (!signedIn) throw fail("signed_out")
        if (next.codePointCount(0, next.length) < MIN_PASSWORD) throw fail("weak_password")
        val keys = authFor(email(), current)
        val salt = AccountKeys.randomBytes(16)
        val kdf = AccountKeys.DEFAULT_KDF
        val fresh = passwordKeys(next, salt, kdf)
        call(
            "POST",
            "/v1/password",
            buildJsonObject {
                put("authKey", AccountKeys.b64u(keys.authKey))
                put("kdf", kdf.toJson())
                put("salt", AccountKeys.b64u(salt))
                put("newAuthKey", AccountKeys.b64u(fresh.authKey))
                put("wrappedKey", AccountKeys.b64u(AccountKeys.wrapDataKey(fresh.wrapKey, dataKey())))
            },
            auth = true,
        )
        return true
    }

    /** A new recovery key; the old one stops working. */
    suspend fun newRecoveryKey(password: String): String {
        if (!signedIn) throw fail("signed_out")
        val keys = authFor(email(), password)
        val recoveryKey = AccountKeys.randomBytes(20)
        val recovery = recoveryKeys(recoveryKey)
        call(
            "POST",
            "/v1/recovery",
            buildJsonObject {
                put("authKey", AccountKeys.b64u(keys.authKey))
                put("recoveryAuth", AccountKeys.b64u(recovery.recoveryAuth))
                put("recoveryWrappedKey", AccountKeys.b64u(AccountKeys.wrapDataKey(recovery.recoveryWrap, dataKey())))
            },
            auth = true,
        )
        return AccountKeys.formatRecoveryKey(recoveryKey)
    }

    /** Deletes the account and everything in it on the server. This phone keeps its own copy. */
    suspend fun deleteAccount(password: String): AccountStatus {
        val session = synchronized(lock) { session } ?: throw fail("signed_out")
        val keys = authFor(email(), password)
        call("DELETE", "/v1/account", buildJsonObject { put("authKey", AccountKeys.b64u(keys.authKey)) }, auth = true, session = session)
        if (endIf(session)) changedStatus()
        return status()
    }

    /** Under the lock. */
    private fun open(token: String, dataKey: ByteArray) {
        val s = Session(token, dataKey, AccountKeys.recordKeys(dataKey))
        session = s
        ids.clear()
        for ((key, e) in entries) {
            val id = AccountKeys.recordId(s.keys.indexKey, e.c, e.k)
            e.id = id
            ids[id] = key
        }
    }

    /**
     * This phone joins the account. Everything it has goes up; what the account has comes down first, so an older
     * value here doesn't overwrite a newer one there.
     */
    private fun attach(email: String, token: String, dataKey: ByteArray) {
        synchronized(lock) {
            state = state.copy(
                email = email,
                sessionServer = state.server ?: defaultServer,
                token = secrets.encrypt(token),
                dataKey = secrets.encrypt(Base64.getEncoder().encodeToString(dataKey)),
                cursor = 0,
                lastSync = null,
                error = null,
            )
            val acct = bookingAcct(email)
            for (e in entries.values) {
                e.seq = null
                // Another Rukoo account's booking page stays with that account: kept here, never sent to this one.
                if (e.c in PER_ACCOUNT && acctOfKey(e.c, e.k) != acct) {
                    e.dirty = false
                    e.ifAbsent = false
                    continue
                }
                e.dirty = true
                // A first write, unless it is a deletion: that settles by its time, like any other.
                if (e.c in FIRST_WRITE_WINS && !e.deleted) {
                    e.ifAbsent = true
                    e.first = true
                }
            }
            open(token, dataKey)
            saveState()
        }
        changedStatus()
        startSync()
        changedStatus()
    }

    /** Under the lock. */
    private fun forget() {
        session = null
        pushJob?.cancel()
        ids.clear()
        // A booking page's private key leaves with the session; signing in again brings it back from the account.
        // Dropped here only, not deleted: a deletion would go up at the next sign-in.
        if (entries.values.removeAll { it.c == "bookingkey" }) saveEntries()
        for (e in entries.values) {
            e.id = null
            e.seq = null
            e.dirty = false
            e.ifAbsent = false
        }
        // A phone pinned to a former default goes back to the default: signed out here, deleted, or ended elsewhere, as
        // when another device deleted the account to make it again at the default.
        state = if (state.server in FORMER_SERVERS) {
            AccountState(error = state.error)
        } else {
            AccountState(server = state.server, info = state.info, error = state.error)
        }
        saveState()
    }

    // ---- records ----

    /** The value of a live record, or null when there is none (JsonNull for a record whose value is null). */
    fun get(c: String, k: String): JsonElement? = synchronized(lock) {
        entries[entryKey(c, k)]?.takeIf { !it.deleted }?.v
    }

    data class Item(val k: String, val v: JsonElement, val t: Long)

    fun list(c: String): List<Item> = synchronized(lock) {
        entries.values.filter { it.c == c && !it.deleted }.map { Item(it.k, it.v, it.t) }
    }

    /**
     * Returns false when [ifAbsent] and the record already exists. An ifAbsent record is a first write: when another
     * device's first write reaches the server before it, that one stays, whatever the times say.
     */
    fun put(c: String, k: String, v: JsonElement, ifAbsent: Boolean = false): Boolean {
        synchronized(lock) {
            val key = entryKey(c, k)
            val was = entries[key]
            if (ifAbsent && was != null && !was.deleted) return false
            val e = AccountEntry(c, k, v, maxOf(clock.now(), if (was != null) was.t + 1 else 0), dirty = true)
            // A first write, unless it takes the place of a deletion this phone knows of: the server keeps that deletion
            // as a record, so a first write would bounce off it. This one settles by its time instead.
            if (ifAbsent && was?.deleted != true) {
                e.ifAbsent = true
                e.first = true
            }
            track(key, e)
        }
        changedRecords(setOf(c))
        return true
    }

    /**
     * A deletion is kept, and goes up, as a record of its own (d: true) with its time, so it settles against a change
     * on another device like any other write, also when it was made while signed out.
     */
    fun remove(c: String, k: String): Boolean {
        synchronized(lock) {
            val key = entryKey(c, k)
            val was = entries[key]
            if (was == null || was.deleted) return false
            track(key, AccountEntry(c, k, JsonNull, maxOf(clock.now(), was.t + 1), deleted = true, dirty = true))
        }
        changedRecords(setOf(c))
        return true
    }

    /** Under the lock. */
    private fun track(key: String, e: AccountEntry) {
        val s = session
        if (s != null) {
            val id = AccountKeys.recordId(s.keys.indexKey, e.c, e.k)
            e.id = id
            ids[id] = key
        } else {
            e.dirty = false
        }
        entries[key] = e
        saveEntries()
        if (s != null) {
            pushJob?.cancel()
            pushJob = scope.launch {
                delay(pushAfterMs)
                runCatching { sync() }
            }
        }
    }

    /**
     * Everything this phone keeps, readable: what "Export my data" saves. Not a booking page's key pair: its private
     * key opens guests' details, and an export is a file anyone may come across. Booking records only of the account
     * signed in now: another Rukoo account's (someone else on a shared phone) hold its guests' names and notes.
     */
    fun exportData(): JsonObject = synchronized(lock) {
        val signedIn = session != null
        val acct = state.email?.takeIf { signedIn }?.let(::bookingAcct)
        buildJsonObject {
            put("exportedAt", iso(clock.now()))
            put("server", if (signedIn) JsonPrimitive(state.server ?: defaultServer) else JsonNull)
            put("email", if (signedIn) JsonPrimitive(state.email) else JsonNull)
            putJsonArray("records") {
                for (e in entries.values) {
                    if (e.deleted || e.c == "bookingkey") continue
                    if (e.c in PER_ACCOUNT && (acct == null || acctOfKey(e.c, e.k) != acct)) continue
                    addJsonObject {
                        put("collection", e.c)
                        put("key", e.k)
                        put("value", e.v)
                        put("changed", iso(e.t))
                    }
                }
            }
        }
    }

    // ---- sync ----

    /**
     * A sync belongs to the session it started in. When this phone signs out or into another account meanwhile,
     * what the server still answers for the old one is dropped, and the new session gets a sync of its own.
     */
    suspend fun sync(): AccountStatus {
        val job = startSync() ?: return status()
        changedStatus()
        return job.await()
    }

    /** The running sync of this session, or a new one; null while signed out. */
    private fun startSync(): Deferred<AccountStatus>? {
        val job = synchronized(lock) {
            val s = session ?: return null
            syncing?.takeIf { it.session === s }?.job ?: scope.async(start = CoroutineStart.LAZY) { runSync(s) }.also { syncing = Syncing(s, it) }
        }
        job.start()
        return job
    }

    /** The sync running for the current session, for tests that need to hold one. */
    internal fun running(): Deferred<AccountStatus>? = synchronized(lock) { syncing?.job }

    private suspend fun runSync(session: Session): AccountStatus {
        fun current() = synchronized(lock) { this.session === session }
        try {
            for (round in 0 until SYNC_ROUNDS) {
                if (!current()) break
                pull(session)
                val refused = push(session)
                if (!refused && synchronized(lock) { entries.values.none { it.dirty } }) break
            }
            synchronized(lock) {
                if (this.session === session) state = state.copy(lastSync = clock.now(), error = null)
            }
        } catch (e: Throwable) {
            synchronized(lock) {
                if (this.session === session && e !is CancellationException) state = state.copy(error = (e as? AccountError)?.code ?: "server")
            }
            throw translated(e)
        } finally {
            synchronized(lock) {
                if (syncing?.session === session) syncing = null
                if (this.session === session) saveState() else saveEntries(now = true)
            }
            changedStatus()
        }
        return status()
    }

    suspend fun pull(session: Session? = synchronized(lock) { this.session }) {
        val touched = HashSet<String>()
        // The cursor moves past each page as it is taken, so a later page that fails doesn't bring this one back:
        // what it changed is announced either way.
        try {
            pullPages(session, touched)
        } finally {
            if (touched.isNotEmpty()) changedRecords(touched)
        }
    }

    private suspend fun pullPages(session: Session?, touched: MutableSet<String>) {
        var restarted = false
        var more = true
        var pages = 0
        val meter = AtomicLong()
        while (more) {
            if (session == null || !synchronized(lock) { this.session === session }) return
            // A server that keeps answering more can't keep the phone busy: what is left comes with the next sync. Every
            // byte of an answer counts, headers and chunk framing too, so padding a page with fields Rukoo ignores or
            // with HTTP framing doesn't get past the limit.
            if (pages >= PULL_PAGES || meter.get() >= PULL_DATA) return
            pages++
            val since = synchronized(lock) { state.cursor }
            val res = call("GET", "/v1/records?since=$since&limit=$PULL_LIMIT", auth = true, session = session, meter = meter) as? JsonObject
            more = synchronized(lock) {
                if (this.session !== session) return
                val records = (res?.get("records") as? JsonArray).orEmpty()
                val top = safeInteger(res?.get("seq"))
                // The server's newest change is older than where this phone got to: it lost data, or an earlier answer
                // was false. Start again from the beginning, once; records already here are taken again without harm.
                if (top != null && top < since) {
                    if (restarted) throw fail("server")
                    restarted = true
                    state = state.copy(cursor = 0)
                    return@synchronized true
                }
                // A page comes after since, in order, and not past the server's newest change; anything else is refused
                // whole, so a false sequence number can't move the cursor past records this phone hasn't seen. So is a
                // page with an id no device could have made, such as a long tombstone id padding the answer.
                if (records.size > PULL_LIMIT) throw fail("server")
                var last = since
                for (r in records) {
                    val o = r as? JsonObject
                    val seq = safeInteger(o?.get("seq"))
                    if (seq == null || seq <= last || (top != null && seq > top)) throw fail("server")
                    val id = (o?.get("id") as? JsonPrimitive)?.takeIf { it.isString }?.content
                    if (id == null || !RECORD_ID.matches(id)) throw fail("server")
                    last = seq
                }
                var cursor = state.cursor
                for (r in records) {
                    val o = r as JsonObject
                    val id = (o["id"] as JsonPrimitive).content
                    val seq = safeInteger(o["seq"]) ?: continue
                    val sealed = (o["data"] as? JsonPrimitive)?.takeIf { it.isString }?.content
                    applyRemote(session, id, seq, sealed)?.let { touched += it }
                    if (seq > cursor) cursor = seq
                }
                state = state.copy(cursor = cursor)
                (res?.get("more") as? JsonPrimitive)?.booleanOrNull == true && records.isNotEmpty()
            }
        }
    }

    /** Where the next pull starts, for tests. */
    internal fun cursor(): Long = synchronized(lock) { state.cursor }

    /** Takes one record from the server; returns its collection when this phone's data changed. Under the lock. */
    private fun applyRemote(session: Session, id: String, seq: Long, data: String?): String? {
        // A tombstone (data null) proves nothing about who wrote it, so a server could use one to erase records here.
        // Devices delete with sealed d: true records, which do prove it; tombstones are left alone.
        if (data == null) return null
        val rec = AccountKeys.openRecord(session.keys, id, data) ?: return null
        val k = entryKey(rec.c, rec.k)
        val mine = entries[k]
        if (mine != null && !mine.dirty && mine.seq == seq) return null
        // Two devices that changed a record at about the same time may reach the server in either order. The later
        // change wins on every device: one that is newer here goes up again, also after it was sent once. A first
        // write (a classification) is the exception: the server keeps whichever came first.
        if (mine != null && !mine.first && newer(mine.t, mine.deleted, mine.v, rec.t, rec.d, rec.v)) {
            mine.dirty = true
            return null
        }
        entries[k] = AccountEntry(rec.c, rec.k, rec.v, rec.t, deleted = rec.d, id = id, seq = seq)
        ids[id] = k
        return rec.c
    }

    /** Sends this phone's changes; true when the server kept another device's record for some of them. */
    suspend fun push(session: Session? = synchronized(lock) { this.session }): Boolean {
        var refused = false
        while (true) {
            if (session == null) return refused
            val (batch, records) = synchronized(lock) {
                if (this.session !== session) return refused
                val dirty = entries.values.filter { it.dirty }.take(PUSH_BATCH)
                if (dirty.isEmpty()) return refused
                // A record too large for the server stays on this phone: sent along, it would fail the whole batch
                // every time.
                val batch = ArrayList<AccountEntry>()
                val records = buildJsonArray {
                    for (e in dirty) {
                        val sealed = AccountKeys.sealRecord(session.keys, Rec(e.c, e.k, if (e.deleted) JsonNull else e.v, e.t, e.deleted))
                        if (AccountKeys.unb64u(sealed.data).size > RECORD_BYTES) {
                            e.dirty = false
                            e.ifAbsent = false
                            e.tooLarge = true
                            continue
                        }
                        batch += e
                        addJsonObject {
                            put("id", sealed.id)
                            put("data", sealed.data)
                            if (e.ifAbsent && !e.deleted) put("ifAbsent", true)
                        }
                    }
                }
                batch to records
            }
            if (records.isEmpty()) {
                synchronized(lock) { saveEntries() }
                continue
            }
            val res = call("POST", "/v1/records", buildJsonObject { put("records", records) }, auth = true, session = session) as? JsonObject
            synchronized(lock) {
                if (this.session !== session) return refused
                val results = (res?.get("results") as? JsonArray).orEmpty().mapNotNull { it as? JsonObject }
                    .associateBy { (it["id"] as? JsonPrimitive)?.content }
                // A server that leaves a record out of its answer would get it again, forever.
                if (batch.any { !results.containsKey(it.id) }) throw fail("server")
                for (e in batch) {
                    val r = results[e.id]
                    // Changed again while this was on its way: it goes up with the next batch.
                    if (r == null || entries[entryKey(e.c, e.k)] !== e) continue
                    e.dirty = false
                    e.ifAbsent = false
                    val seq = (r["seq"] as? JsonPrimitive)?.content?.toDoubleOrNull()?.toLong() ?: 0L
                    if ((r["stored"] as? JsonPrimitive)?.booleanOrNull == true) {
                        e.seq = seq
                        continue
                    }
                    // Another device's first write came first. The next pull must bring it, even when it is older than
                    // the cursor.
                    refused = true
                    e.seq = null
                    state = state.copy(cursor = minOf(state.cursor, seq - 1))
                }
                saveEntries()
            }
        }
    }

    // ---- persistence and events ----

    /**
     * Under the lock. The records go first: a cursor on disk past records that aren't would have them skipped for good
     * after a restart.
     */
    private fun saveState() {
        saveEntries(now = true)
        // When the records couldn't be written (a full disk), the cursor on disk stays where they last were.
        val saved = state.copy(cursor = minOf(state.cursor, durableCursor))
        runCatching { JsonFiles.write(stateFile, AccountState.serializer(), saved) }.onFailure { AppLog.w(TAG, "could not save the account", it) }
    }

    /** Under the lock; [now] writes at once (false when that failed), else a second after the last change. */
    private fun saveEntries(now: Boolean = false): Boolean {
        saveJob?.cancel()
        saveJob = null
        if (now) return writeEntries()
        saveJob = scope.launch {
            delay(saveAfterMs)
            synchronized(lock) { writeEntries() }
        }
        return true
    }

    /** Under the lock. False when the disk refused the records; they are tried again a little later then. */
    private fun writeEntries(): Boolean {
        val copy = entries.values.map { AccountEntry(it.c, it.k, it.v, it.t, it.deleted, it.dirty, it.ifAbsent, it.first, it.id, it.seq, it.tooLarge) }
        val cursor = state.cursor
        val written = runCatching { JsonFiles.write(recordsFile, StoredEntries.serializer(), StoredEntries(copy)) }
            .onFailure { AppLog.w(TAG, "could not save the account's records", it) }
            .isSuccess
        if (written) {
            durableCursor = cursor
        } else {
            saveJob = scope.launch {
                delay(SAVE_RETRY)
                synchronized(lock) { writeEntries() }
            }
        }
        return written
    }

    /** Writes the records that wait for their save now; false when they couldn't be written. */
    fun flush(): Boolean = synchronized(lock) {
        if (saveJob != null) saveEntries(now = true) else true
    }

    /** Writes what waits and stops the timers, for quitting and for tests. */
    fun close() = synchronized(lock) {
        pushJob?.cancel()
        if (saveJob != null) saveEntries(now = true)
    }

    private fun changedStatus() {
        _status.value = status()
    }

    private fun changedRecords(collections: Set<String>) {
        _version.value++
        _records.tryEmit(collections)
        changedStatus()
    }

    companion object {
        const val PROTOCOL = 1
        const val MIN_PASSWORD = 12
        const val SYNC_EVERY = 5 * 60 * 1000L
        const val PUSH_AFTER = 2000L
        const val SAVE_AFTER = 1000L
        private const val SAVE_RETRY = 30_000L
        private const val PULL_LIMIT = 1000
        private const val PUSH_BATCH = 500

        // When one pull stops asking for more: after 500 pages of 1000 records and tombstones, or once 256 MiB of answers
        // counted whole, with their headers, framing and the fields Rukoo ignores, have come in. The meter is checked
        // between pages, so the page that crosses it is the last. A full account is about 170 MiB of them
        // (server/PROTOCOL.md).
        internal const val PULL_PAGES = 500
        internal const val PULL_DATA = 256L * 1024 * 1024

        // A record id is an HMAC-SHA256 in base64url (server/PROTOCOL.md, Records).
        private val RECORD_ID = Regex("[A-Za-z0-9_-]{43}")

        // The most a record may take on the server, once sealed (server/PROTOCOL.md).
        private const val RECORD_BYTES = 16384

        // Rounds of pull and push in one sync, enough for a refused first write or a newer value that has to go up again.
        private const val SYNC_ROUNDS = 3

        // Collections whose records are decided once, by the first device to write one (server/PROTOCOL.md, Collections).
        private val FIRST_WRITE_WINS = setOf("classification")

        // Collections whose keys name the Rukoo account they belong to (<acct>): a booking page's records, which desktop
        // writes and this phone receives with the rest of the account.
        private val PER_ACCOUNT = setOf("booking", "bookingkey")
        private val ACCT_IN_KEY = Regex("^[a-z]+:([A-Za-z0-9_-]{16})(?::|$)")

        // Addresses Rukoo used as its default server before. A phone signed in at one keeps it ([pin]) while the session
        // lasts; once it ends, the phone goes back to the default ([forget]).
        internal val FORMER_SERVERS = listOf("https://rukoo-account.bvdm.ai")

        /** `<acct>`: the first 16 characters of base64url(SHA-256) of the trimmed, lowercased Rukoo account email. */
        fun bookingAcct(email: String): String =
            AccountKeys.b64u(java.security.MessageDigest.getInstance("SHA-256").digest(AccountKeys.normalizeEmail(email).toByteArray(Charsets.UTF_8))).take(16)

        /** The `<acct>` a booking record's key names: bookingkey/<acct>, booking/settings:<acct>, booking/map:<acct>:<id>. */
        internal fun acctOfKey(c: String, k: String): String? = if (c == "bookingkey") k else ACCT_IN_KEY.find(k)?.groupValues?.get(1)

        private const val TAG = "Account"

        // JavaScript's \s, which takes in Unicode spaces too.
        private const val SPACE = "\\t\\n\\u000B\\f\\r \\u00A0\\u1680\\u2000-\\u200A\\u2028\\u2029\\u202F\\u205F\\u3000\\uFEFF"
        private val EMAIL = Regex("^[^@$SPACE]+@[^@$SPACE]+\\.[^@$SPACE]+$")

        /** Number(v) when it is a safe integer, as JavaScript's Number.isSafeInteger takes it; else null. */
        private fun safeInteger(v: JsonElement?): Long? {
            val d = (v as? JsonPrimitive)?.takeIf { it !is JsonNull }?.content?.trim()?.toDoubleOrNull() ?: return null
            return if (d % 1.0 == 0.0 && kotlin.math.abs(d) <= 9007199254740991.0) d.toLong() else null
        }

        fun validEmail(email: String): Boolean = EMAIL.matches(email) && email.length <= 320

        private fun entryKey(c: String, k: String) = "$c\n$k"

        /**
         * Which of two versions of a record wins: the later change, and on a tie the larger JSON of [d, v], so every
         * device picks the same one whatever order they arrive in.
         */
        fun newer(aT: Long, aGone: Boolean, aV: JsonElement, bT: Long, bGone: Boolean, bV: JsonElement): Boolean {
            if (aT != bT) return aT > bT
            val a = JsJson.stringify(buildJsonArray { add(if (aGone) 1 else 0); add(aV) })
            val b = JsJson.stringify(buildJsonArray { add(if (bGone) 1 else 0); add(bV) })
            return a > b
        }

        private val ISO = DateTimeFormatter.ofPattern("yyyy-MM-dd'T'HH:mm:ss.SSS'Z'").withZone(ZoneOffset.UTC)

        /** `new Date(ms).toISOString()`. */
        fun iso(ms: Long): String = ISO.format(Instant.ofEpochMilli(ms))
    }
}
