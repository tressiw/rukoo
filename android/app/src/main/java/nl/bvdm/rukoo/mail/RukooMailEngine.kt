package nl.bvdm.rukoo.mail

import java.io.File
import java.io.IOException
import java.security.MessageDigest
import java.time.ZoneId
import java.util.Locale
import java.util.concurrent.ConcurrentHashMap
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Deferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.Job
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.async
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.flowOn
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.serialization.builtins.ListSerializer
import kotlinx.serialization.builtins.MapSerializer
import kotlinx.serialization.builtins.serializer
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.booleanOrNull
import nl.bvdm.rukoo.R
import nl.bvdm.rukoo.core.AppLog
import nl.bvdm.rukoo.core.Clock
import nl.bvdm.rukoo.core.Ids
import nl.bvdm.rukoo.core.JsonFiles
import nl.bvdm.rukoo.core.SecretCipher
import nl.bvdm.rukoo.mail.compose.ComposeBody
import nl.bvdm.rukoo.mail.compose.ComposerFields
import nl.bvdm.rukoo.mail.compose.QuoteLabels
import nl.bvdm.rukoo.mail.compose.ReplyBuilder
import nl.bvdm.rukoo.mail.demo.DemoSession
import nl.bvdm.rukoo.mail.engine.AccountCache
import nl.bvdm.rukoo.mail.engine.CacheOps
import nl.bvdm.rukoo.mail.engine.CachedBox
import nl.bvdm.rukoo.mail.engine.Limits
import nl.bvdm.rukoo.mail.engine.MailSnapshot
import nl.bvdm.rukoo.mail.engine.MailViews
import nl.bvdm.rukoo.mail.engine.PendingPatch
import nl.bvdm.rukoo.mail.google.GoogleAuth
import nl.bvdm.rukoo.mail.html.MailHtml
import nl.bvdm.rukoo.mail.html.SignatureHtml
import nl.bvdm.rukoo.mail.mime.ComposedMail
import nl.bvdm.rukoo.mail.mime.MailAttachment
import nl.bvdm.rukoo.mail.mime.MailText
import nl.bvdm.rukoo.mail.mime.MimeComposer
import nl.bvdm.rukoo.mail.mime.MimeParser
import nl.bvdm.rukoo.mail.mime.ParsedMail
import nl.bvdm.rukoo.mail.model.Account
import nl.bvdm.rukoo.mail.model.AccountPatch
import nl.bvdm.rukoo.mail.model.AccountType
import nl.bvdm.rukoo.mail.model.MailProviders
import nl.bvdm.rukoo.mail.model.Address
import nl.bvdm.rukoo.mail.model.Alias
import nl.bvdm.rukoo.mail.model.AttachmentContent
import nl.bvdm.rukoo.mail.model.AttachmentInfo
import nl.bvdm.rukoo.mail.model.AuthKind
import nl.bvdm.rukoo.mail.model.CachedMessage
import nl.bvdm.rukoo.mail.model.DeleteOutcome
import nl.bvdm.rukoo.mail.model.DeleteResult
import nl.bvdm.rukoo.mail.model.Failed
import nl.bvdm.rukoo.mail.model.Folder
import nl.bvdm.rukoo.mail.model.FolderRole
import nl.bvdm.rukoo.mail.model.FullMessage
import nl.bvdm.rukoo.mail.model.GoogleClient
import nl.bvdm.rukoo.mail.model.GoogleTokens
import nl.bvdm.rukoo.mail.model.Identity
import nl.bvdm.rukoo.mail.model.ImapAccountInput
import nl.bvdm.rukoo.mail.model.ImportedAccount
import nl.bvdm.rukoo.mail.model.MailCounts
import nl.bvdm.rukoo.mail.model.MailEvent
import nl.bvdm.rukoo.mail.model.MailPrefs
import nl.bvdm.rukoo.mail.model.MailView
import nl.bvdm.rukoo.mail.model.MessageIds
import nl.bvdm.rukoo.mail.model.MessagePin
import nl.bvdm.rukoo.mail.model.MessageSummary
import nl.bvdm.rukoo.mail.model.MoveResult
import nl.bvdm.rukoo.mail.model.Moved
import nl.bvdm.rukoo.mail.model.OutgoingMessage
import nl.bvdm.rukoo.mail.model.ReplyKind
import nl.bvdm.rukoo.mail.model.Signature
import nl.bvdm.rukoo.mail.model.SortOrder
import nl.bvdm.rukoo.mail.model.SyncStatus
import nl.bvdm.rukoo.mail.model.UndoResult
import nl.bvdm.rukoo.mail.model.UnsubscribeResult
import nl.bvdm.rukoo.mail.model.ViewKind
import nl.bvdm.rukoo.mail.net.SafeHttp
import nl.bvdm.rukoo.mail.session.Credentials
import nl.bvdm.rukoo.mail.session.FriendlyErrors
import nl.bvdm.rukoo.mail.session.ImapSession
import nl.bvdm.rukoo.mail.session.MailSession
import nl.bvdm.rukoo.mail.session.UidValidityChanged
import nl.bvdm.rukoo.ui.Mailto
import okhttp3.OkHttpClient
import okhttp3.Request

/** Creates the session for an account; tests replace it with fakes. */
fun interface SessionFactory {
    fun create(account: Account, credentials: Credentials): MailSession
}

/**
 * The phone's mail engine, a port of desktop's Engine (src/main/engine.js, spec 4). Lists and counts
 * come from a JSON cache of message headers per account; bodies are fetched on demand and kept as
 * .eml files; every change is optimistic in the cache first and reverts when the server refuses.
 *
 * All state changes run on one serial dispatcher, so the code keeps desktop's single-threaded
 * reasoning: interleaving happens only where a call suspends, like an `await` on desktop. Files live
 * under [dir]: accounts.json, secrets.json (SecretBox-encrypted passwords and refresh tokens),
 * prefs.json, cache/<id>.json, bodies/<id>/<sha1>.eml and demo-server.json.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class RukooMailEngine(
    private val dir: File,
    /** Where attachments are written for opening and sharing (cache/attachments for the FileProvider). */
    private val attachmentsDir: File,
    private val scope: CoroutineScope,
    private val cipher: SecretCipher,
    private val strings: MailStrings,
    private val http: OkHttpClient,
    private val clock: Clock = Clock.System,
    private val googleAuth: GoogleAuth? = null,
    sessionFactory: SessionFactory? = null,
    private val safeHttp: SafeHttp = SafeHttp(http),
    private val zone: () -> ZoneId = { ZoneId.systemDefault() },
    private val locale: () -> Locale = { Locale.getDefault() },
    private val saveDelayMs: Long = Limits.CACHE_SAVE_DELAY_MS,
    /** Called after an account is added, to make the first one the default. */
    private val onAccountAdded: suspend (accountId: String) -> Unit = {},
    /** Called after an account is removed, with the account that remains first. */
    private val onAccountRemoved: suspend (accountId: String, firstLeft: String?) -> Unit = { _, _ -> },
    /** Fresh unread inbox mail found by a sync, for notifications. Runs on the engine's thread. */
    private val onNewMail: (List<MessageSummary>) -> Unit = {},
    /** Called when the set of accounts changes, to start or stop background sync. */
    private val onAccountsChanged: (List<Account>) -> Unit = {},
) : MailEngine {
    private val engine = Dispatchers.Default.limitedParallelism(1)
    private val ready = CompletableDeferred<Unit>()

    private val factory: SessionFactory = sessionFactory ?: SessionFactory { account, credentials ->
        if (account.type == AccountType.DEMO) DemoSession(File(dir, DemoSession.FILE_NAME), strings, clock, zone()) else ImapSession(account, credentials, strings)
    }

    private val _accounts = MutableStateFlow<List<Account>>(emptyList())
    override val accounts: StateFlow<List<Account>> = _accounts.asStateFlow()

    private val _loaded = MutableStateFlow(false)
    override val loaded: StateFlow<Boolean> = _loaded.asStateFlow()

    private val caches = MutableStateFlow<Map<String, AccountCache>>(emptyMap())
    private val _prefs = MutableStateFlow(MailPrefs())
    override val prefs: StateFlow<MailPrefs> = _prefs.asStateFlow()

    private val snapshot: StateFlow<MailSnapshot> =
        combine(_accounts, caches, _prefs) { a, c, p -> MailSnapshot(a, c, p) }.stateIn(scope, SharingStarted.Eagerly, MailSnapshot())

    override val folders: StateFlow<Map<String, List<Folder>>> = snapshot.map { s ->
        s.accounts.associate { acc ->
            val cache = s.caches[acc.id] ?: AccountCache()
            acc.id to cache.folders.map { f ->
                val box = cache.boxes[f.path]
                Folder(acc.id, f.path, f.name, f.role, f.delimiter, f.specialUse, box?.messages?.count { it.unread } ?: 0, box?.messages?.size ?: 0)
            }
        }
    }.distinctUntilChanged().flowOn(Dispatchers.Default).stateIn(scope, SharingStarted.Eagerly, emptyMap())

    private val running = MutableStateFlow<Set<String>>(emptySet())
    override val sync: StateFlow<SyncStatus> = combine(running, caches) { r, c ->
        SyncStatus(
            running = r,
            errors = c.mapNotNull { (id, cache) -> cache.error?.let { id to it } }.toMap(),
            lastSync = c.mapNotNull { (id, cache) -> cache.lastSync?.let { id to it } }.toMap(),
        )
    }.stateIn(scope, SharingStarted.Eagerly, SyncStatus())

    override val unreadCount: StateFlow<Int> =
        snapshot.map { MailViews.unreadCount(it) }.distinctUntilChanged().flowOn(Dispatchers.Default).stateIn(scope, SharingStarted.Eagerly, 0)

    private val _events = MutableSharedFlow<MailEvent>(extraBufferCapacity = 64)
    override val events: SharedFlow<MailEvent> = _events

    // Engine-thread state, like the fields of desktop's Engine.
    private var secrets: Map<String, String> = emptyMap()
    private val sessions = mutableMapOf<String, MailSession>()
    private val syncJobs = mutableMapOf<String, Deferred<List<MessageSummary>>>()
    private val pending = mutableMapOf<String, PendingPatch>()
    private val saveJobs = mutableMapOf<String, SaveJob>()
    private val writeLocks = ConcurrentHashMap<String, Mutex>()
    private val flagLocks = ConcurrentHashMap<String, Mutex>()
    private val downloads = mutableMapOf<String, Download>()
    private val prefetching = mutableSetOf<String>()
    private val tokens = mutableMapOf<String, TokenEntry>()
    private val parsedCache = object : LinkedHashMap<String, ParsedMail>(8, 0.75f, true) {
        override fun removeEldestEntry(eldest: MutableMap.MutableEntry<String, ParsedMail>?) = size > 4
    }

    /** Recent moves by the id the message had before, so they can be reversed (read by canUndo off the engine thread). */
    private val undoable = ConcurrentHashMap<String, UndoRecord>()

    @Volatile
    private var closed = false

    private class UndoRecord(
        val accountId: String,
        val folder: String,
        val destination: String,
        val uid: Long,
        val uidValidity: String,
        val msg: CachedMessage,
        val at: Long,
    )

    private class Download(val background: Boolean, val raw: Deferred<ByteArray>)

    /** A debounced cache save; once [writing], it is no longer cancelled but waited for. */
    private class SaveJob(val job: Job) {
        @Volatile
        var writing = false
    }

    private class TokenEntry(val accessToken: String?, val expiresAt: Long, val refreshing: Deferred<String>? = null)

    private val accountsFile get() = File(dir, "accounts.json")
    private val secretsFile get() = File(dir, "secrets.json")
    private val prefsFile get() = File(dir, "prefs.json")

    private fun cacheFile(id: String) = File(dir, "cache/$id.json")

    private fun bodiesDir(id: String) = File(dir, "bodies/$id")

    init {
        scope.launch(engine) {
            try {
                val stored = withContext(Dispatchers.IO) { load() }
                // Published only when every file read: an engine started from half its data would write over the rest.
                secrets = stored.secrets
                _prefs.value = stored.prefs
                caches.value = stored.caches
                _accounts.value = stored.accounts
                ready.complete(Unit)
                _loaded.value = true
                onAccountsChanged(_accounts.value)
            } catch (e: Exception) {
                // accounts.json, secrets.json or prefs.json exists but can't be read. Every call fails with this
                // message and [loaded] stays false, so nothing is written over the files.
                AppLog.e(TAG, "could not read the mail data", e)
                ready.completeExceptionally(MailException(strings.get(R.string.engine_errors_storage), "storage", e))
            }
        }
    }

    private class Stored(val accounts: List<Account>, val secrets: Map<String, String>, val prefs: MailPrefs, val caches: Map<String, AccountCache>)

    private fun load(): Stored {
        val list = JsonFiles.read(accountsFile, ListSerializer(Account.serializer()), emptyList())
        val secrets = JsonFiles.read(secretsFile, MapSerializer(String.serializer(), String.serializer()), emptyMap())
        val prefs = JsonFiles.read(prefsFile, MailPrefs.serializer(), MailPrefs())
        // A cache that can't be read starts empty: it only holds what the next sync fetches again.
        val loadedCaches = list.associate { acc ->
            acc.id to (runCatching { JsonFiles.read(cacheFile(acc.id), AccountCache.serializer(), AccountCache()) }.getOrNull() ?: AccountCache())
        }
        // Stored signatures are cleaned again, in case a file was edited by hand (engine.js:172-173).
        return Stored(
            accounts = list.map { acc -> acc.signatureHtml?.let { acc.copy(signatureHtml = SignatureHtml.reclean(it).ifEmpty { null }) } ?: acc },
            secrets = secrets,
            prefs = prefs.copy(signatureHtml = SignatureHtml.reclean(prefs.signatureHtml)),
            caches = loadedCaches,
        )
    }

    /** Runs [block] on the engine's thread once the data is loaded. */
    private suspend fun <T> on(block: suspend () -> T): T = withContext(engine) {
        ready.await()
        block()
    }

    /**
     * A change to mail or accounts: like [on], but it runs to the end once started. A caller that goes away
     * (a closed screen) must not leave an optimistic change in the cache that never reached the server.
     */
    private suspend fun <T> change(block: suspend () -> T): T = withContext(engine + NonCancellable) {
        ready.await()
        block()
    }

    // ---------- persistence ----------

    private suspend fun persistAccounts() = withContext(Dispatchers.IO) {
        JsonFiles.write(accountsFile, ListSerializer(Account.serializer()), _accounts.value)
        JsonFiles.write(secretsFile, MapSerializer(String.serializer(), String.serializer()), secrets)
    }

    private fun writeLock(accountId: String) = writeLocks.getOrPut(accountId) { Mutex() }

    /**
     * Saves the account's cache shortly after the last change (engine.js persistCache, 250 ms). Writes of
     * one account take turns under its lock and each writes the cache as it is then, so the newest wins.
     */
    private fun persistCache(accountId: String) {
        saveJobs[accountId]?.let { if (!it.writing) it.job.cancel() }
        lateinit var entry: SaveJob
        val job = scope.launch(engine, start = CoroutineStart.LAZY) {
            try {
                delay(saveDelayMs)
                entry.writing = true
                writeLock(accountId).withLock { writeCache(accountId) }
            } finally {
                if (saveJobs[accountId] === entry) saveJobs.remove(accountId)
            }
        }
        entry = SaveJob(job)
        saveJobs[accountId] = entry
        job.start()
    }

    private suspend fun writeCache(accountId: String) {
        // Read at write time: a removed account writes nothing, and a later change is never overtaken by an older one.
        val cache = caches.value[accountId] ?: return
        // A save that fails here is made again with the next change.
        withContext(Dispatchers.IO + NonCancellable) {
            runCatching { JsonFiles.write(cacheFile(accountId), AccountCache.serializer(), cache) }
                .onFailure { AppLog.w(TAG, "could not save a mail cache", it) }
        }
    }

    /** Writes every cache now, after any save already under way; for tests and shutdown. */
    suspend fun flush() = on {
        for (entry in saveJobs.values.toList()) if (!entry.writing) entry.job.cancel()
        for (id in caches.value.keys) writeLock(id).withLock { writeCache(id) }
    }

    private fun mutate(accountId: String, persist: Boolean = true, change: (AccountCache) -> AccountCache) {
        caches.update { all -> all[accountId]?.let { all + (accountId to change(it)) } ?: all }
        if (persist) persistCache(accountId)
    }

    // ---------- accounts and sessions ----------

    private fun accountOrNull(id: String) = _accounts.value.firstOrNull { it.id == id }

    private fun account(id: String): Account = accountOrNull(id) ?: strings.fail(R.string.engine_errors_account_missing, code = "account")

    private fun friendly(e: Throwable) = FriendlyErrors.message(e, strings)

    private fun credentials(acc: Account, plainSecret: String? = null): Credentials =
        if (acc.auth == AuthKind.OAUTH2) {
            Credentials.OAuth { accessToken(acc.id, plainSecret) }
        } else {
            Credentials.Password(plainSecret ?: secrets[acc.id]?.let(cipher::decrypt).orEmpty())
        }

    private fun session(acc: Account): MailSession = sessions.getOrPut(acc.id) { factory.create(acc, credentials(acc)) }

    /**
     * A valid Google access token: the cached one while it has more than a minute left, else a refresh
     * with the stored refresh token. A refresh writes back only while it still owns the entry, so signing
     * in again during it wins (engine.js accessToken).
     */
    private suspend fun accessToken(accountId: String, refreshToken: String? = null): String = withContext(engine) {
        val cached = tokens[accountId]
        if (cached?.accessToken != null && cached.expiresAt - clock.now() > 60_000) return@withContext cached.accessToken
        cached?.refreshing?.let { return@withContext it.await() }
        val google = googleAuth ?: strings.fail(R.string.engine_errors_google_unavailable, code = "no_client")
        val secret = refreshToken ?: secrets[accountId]?.let(cipher::decrypt) ?: strings.fail(R.string.engine_errors_google_expired, code = "invalid_grant")
        lateinit var entry: TokenEntry
        val job = scope.async(engine, start = CoroutineStart.LAZY) {
            val grant = google.refresh(secret)
            if (tokens[accountId] === entry) {
                tokens[accountId] = TokenEntry(grant.accessToken, grant.expiresAt)
                grant.scopes?.let { setScopes(accountId, it) }
            }
            grant.accessToken
        }
        entry = TokenEntry(null, 0, job)
        tokens[accountId] = entry
        try {
            job.await()
        } catch (e: Exception) {
            if (tokens[accountId] === entry) tokens.remove(accountId)
            throw e
        }
    }

    private suspend fun setScopes(accountId: String, scopes: List<String>) {
        val next = scopes.distinct().sorted()
        val acc = accountOrNull(accountId) ?: return
        if (acc.scopes == next) return
        replaceAccount(acc.copy(scopes = next))
        persistAccounts()
    }

    private fun replaceAccount(acc: Account) = _accounts.update { list -> list.map { if (it.id == acc.id) acc else it } }

    override fun identities(accountId: String): List<Identity> = accountOrNull(accountId)?.let(MailViews::identities).orEmpty()

    override fun signatureFor(accountId: String): Signature? {
        val acc = accountOrNull(accountId) ?: return null
        val general = acc.signature == null && acc.signatureHtml.isNullOrEmpty()
        val prefs = _prefs.value
        val text = if (general) prefs.signature else acc.signature.orEmpty()
        val html = if (general) prefs.signatureHtml else acc.signatureHtml.orEmpty()
        val shownText = if (html.isNotEmpty()) SignatureHtml.text(html) else text
        if (shownText.isBlank() && html.isEmpty()) return null
        return Signature(shownText, html.ifEmpty { MailText.esc(text).replace("\n", "<br>") })
    }

    override fun archiveFolder(accountId: String): String? = caches.value[accountId]?.archiveFolder()?.path

    // ---------- sync ----------

    override suspend fun refresh(accountId: String?) = on {
        if (accountId != null) {
            try {
                syncAccount(accountId).await()
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                throw FriendlyErrors.wrap(e, strings)
            }
            return@on
        }
        val jobs = _accounts.value.map { it to syncAccount(it.id) }
        val errors = mutableListOf<String>()
        for ((acc, job) in jobs) {
            try {
                job.await()
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                errors += "${acc.email}: ${friendly(e)}"
            }
        }
        if (errors.isNotEmpty()) throw MailException(errors.joinToString("\n"), "sync")
    }

    /** The running sync of an account, or a new one (engine.js syncAccount). Runs on the engine thread. */
    private fun syncAccount(id: String, extraFolders: List<String> = emptyList()): Deferred<List<MessageSummary>> {
        syncJobs[id]?.takeIf { it.isActive }?.let { return it }
        val job = scope.async(engine, start = CoroutineStart.LAZY) {
            try {
                runSync(id, extraFolders)
            } finally {
                syncJobs.remove(id)
                running.update { it - id }
            }
        }
        syncJobs[id] = job
        running.update { it + id }
        job.start()
        return job
    }

    private suspend fun runSync(id: String, extraFolders: List<String>): List<MessageSummary> {
        var acc = accountOrNull(id) ?: return emptyList()
        // Pick up Gmail aliases once for Google accounts; failures here never block mail sync.
        if (acc.auth == AuthKind.OAUTH2 && acc.aliasesFetchedAt == null) {
            try {
                fetchGmailAliasesNow(id)
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                AppLog.w(TAG, "Gmail aliases unavailable", e)
            }
        }
        // An account removed during that wait must not get a new session.
        acc = accountOrNull(id) ?: return emptyList()
        val fresh = mutableListOf<MessageSummary>()
        var failed: Exception? = null
        try {
            val listed = session(acc).listFolders(background = true)
            mutate(id, persist = false) { it.copy(folders = listed) }
            val cache = caches.value[id] ?: return emptyList()
            val targets = cache.folders.filter { it.role in Limits.SYNC_ROLES }.map { it.path }.toMutableList()
            cache.archiveFolder()?.path?.let { if (it !in targets) targets += it }
            // User folders opened before stay in sync too.
            for (p in extraFolders + cache.boxes.keys) {
                if (p !in targets && cache.folder(p) != null) targets += p
            }
            for (p in targets) {
                try {
                    fresh += syncFolderInto(acc, p, background = true)
                } catch (e: CancellationException) {
                    throw e
                } catch (e: Exception) {
                    // A folder that fails leaves the others to sync. Not once the engine closed or the account went:
                    // the next folder would open a new session for it.
                    if (closed || accountOrNull(id) == null) throw e
                    AppLog.w(TAG, "folder sync failed", e)
                    if (failed == null) failed = e
                }
            }
            // The last sync stays the last one that got through every folder.
            val error = failed?.let { friendly(it) }
            val complete = failed == null
            mutate(id) { c -> c.copy(boxes = c.boxes.filterKeys { k -> c.folder(k) != null }, lastSync = if (complete) clock.now() else c.lastSync, error = error) }
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            mutate(id) { it.copy(error = friendly(e)) }
            throw e
        }
        // New mail in the folders that did sync is announced now: the next sync no longer sees it as new.
        if (fresh.isNotEmpty()) announce(fresh)
        scope.launch(engine) { prefetch(id) }
        failed?.let { throw it }
        return fresh
    }

    private fun announce(fresh: List<MessageSummary>) {
        _events.tryEmit(MailEvent.NewMail(fresh))
        runCatching { onNewMail(fresh) }.onFailure { AppLog.w(TAG, "new-mail listener failed", it) }
    }

    /** Syncs one folder into the cache; returns new inbox mail (engine.js syncFolderInto). */
    private suspend fun syncFolderInto(acc: Account, path: String, background: Boolean = false): List<MessageSummary> {
        val cache = caches.value[acc.id] ?: return emptyList()
        val info = cache.folder(path)
        val prev = cache.boxes[path]
        val known = prev?.messages?.associateBy { it.uid }.orEmpty()
        val limit = if (info?.role == FolderRole.INBOX) Limits.INBOX else Limits.DEFAULT
        val inbox = info?.role == FolderRole.INBOX
        // What inbox categories sort by (categories/): the headers that say what kind of mail it is, and Gmail's tab. The
        // inbox also asks how much older mail is unread, which its unread count adds (MailViews.olderUnread).
        val res = session(acc).syncFolder(
            path, limit, known, prev?.uidValidity, background,
            signals = inbox, gmailTabs = inbox && acc.provider == "google", olderUnseen = inbox,
        )
        val sameValidity = prev != null && prev.uidValidity == res.uidValidity
        mutate(acc.id, persist = false) { c ->
            val messages = CacheOps.applyPending(acc.id, path, res.messages, pending)
            c.copy(boxes = c.boxes + (path to CachedBox(res.uidValidity, res.exists, clock.now(), messages, res.olderUnseen.takeIf { inbox })))
        }
        if (!sameValidity && prev != null) {
            // Old uids point at other messages now: their bodies and undo records are worthless.
            withContext(Dispatchers.IO) { bodiesDir(acc.id).deleteRecursively() }
            undoable.entries.removeIf { it.value.accountId == acc.id && it.value.destination == path }
        }
        val prefs = _prefs.value
        return CacheOps.freshMail(prev, res, info?.role == FolderRole.INBOX) { MailViews.isSpam(prefs, it.from) }
            .map { MailViews.publicMessage(acc, path, info, it, prefs) }
    }

    /** For syncs outside runSync (after a move, opening a folder): still reports new mail. */
    private suspend fun syncFolderAndNotify(acc: Account, path: String) {
        val fresh = syncFolderInto(acc, path)
        persistCache(acc.id)
        if (fresh.isNotEmpty()) announce(fresh)
    }

    private fun resyncLater(acc: Account, path: String) {
        scope.launch(engine) {
            try {
                if (accountOrNull(acc.id) != null) syncFolderAndNotify(acc, path)
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                AppLog.w(TAG, "folder sync failed", e)
            }
        }
    }

    override suspend fun openFolder(accountId: String, path: String) = on {
        val box = caches.value[accountId]?.boxes?.get(path)
        if (box != null && clock.now() - box.syncedAt < Limits.OPEN_FOLDER_FRESH_MS) return@on
        try {
            syncFolderAndNotify(account(accountId), path)
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            throw FriendlyErrors.wrap(e, strings)
        }
    }

    // ---------- lists ----------

    override fun messages(view: MailView, query: String, sort: SortOrder): Flow<List<MessageSummary>> =
        snapshot.map { MailViews.list(it, view, query, sort, locale()) }.distinctUntilChanged().flowOn(Dispatchers.Default)

    override suspend fun listMessages(view: MailView, query: String, sort: SortOrder): List<MessageSummary> = on {
        MailViews.list(live(), view, query, sort, locale())
    }

    /** The state as it is now, not the snapshot stateIn last published. */
    private fun live() = MailSnapshot(_accounts.value, caches.value, _prefs.value)

    override fun counts(accountId: String?): Flow<MailCounts> =
        snapshot.map { MailViews.counts(it, accountId) }.distinctUntilChanged().flowOn(Dispatchers.Default)

    override suspend fun contacts(): List<Address> = on { MailViews.contacts(live()) }

    override suspend fun summary(id: String): MessageSummary? = on { locateOrNull(id)?.let { publicOf(it) } }

    override suspend fun findByMessageId(messageId: String, accountId: String?): List<MessageSummary> =
        on { MailViews.findByMessageId(live(), messageId, accountId) }

    /**
     * The current id of a message Rukoo moved, found by its Message-ID in the folder it went to; that
     * folder is synced first when its copy isn't cached yet (engine.js findMoved).
     */
    override suspend fun findMoved(accountId: String, messageId: String, fromFolder: String?): String? = on {
        val cache = caches.value[accountId] ?: return@on null
        val last = CacheOps.lastMove(cache, messageId, fromFolder) ?: return@on null
        var uid = CacheOps.uidByMessageId(cache, last.folder, messageId)
        if (uid == null) {
            runCatching { openFolderNow(accountId, last.folder) }
            uid = caches.value[accountId]?.let { CacheOps.uidByMessageId(it, last.folder, messageId) }
        }
        uid?.let { MessageIds.encode(accountId, last.folder, it) }
    }

    private suspend fun openFolderNow(accountId: String, path: String) {
        val box = caches.value[accountId]?.boxes?.get(path)
        if (box != null && clock.now() - box.syncedAt < Limits.OPEN_FOLDER_FRESH_MS) return
        syncFolderAndNotify(account(accountId), path)
    }

    override suspend fun pinOf(id: String): MessagePin? = on { locateOrNull(id)?.let { MessagePin(it.msg.messageId, it.box.uidValidity) } }

    // ---------- messages ----------

    private class Located(val acc: Account, val folder: String, val uid: Long, val box: CachedBox, val msg: CachedMessage, val id: String)

    private fun locateOrNull(id: String): Located? {
        val key = MessageIds.decode(id) ?: return null
        val acc = accountOrNull(key.accountId) ?: return null
        val box = caches.value[acc.id]?.boxes?.get(key.folder) ?: return null
        val msg = box.messages.firstOrNull { it.uid == key.uid } ?: return null
        return Located(acc, key.folder, key.uid, box, msg, id)
    }

    private fun locate(id: String): Located = locateOrNull(id) ?: strings.fail(R.string.engine_errors_message_missing, code = "missing")

    private fun publicOf(l: Located) = MailViews.publicMessage(l.acc, l.folder, caches.value[l.acc.id]?.folder(l.folder), l.msg, _prefs.value)

    /** Where a message's raw source is kept once downloaded: sha1 of folder, UIDVALIDITY and uid. */
    private fun bodyFile(accountId: String, folder: String, uidValidity: String, uid: Long): File {
        val digest = MessageDigest.getInstance("SHA-1").digest("$folder|$uidValidity|$uid".toByteArray(Charsets.UTF_8))
        return File(bodiesDir(accountId), digest.joinToString("") { "%02x".format(it) } + ".eml")
    }

    override suspend fun rawSource(id: String): ByteArray = on { rawSourceNow(locate(id)) }

    private suspend fun rawSourceNow(l: Located): ByteArray {
        val file = bodyFile(l.acc.id, l.folder, l.box.uidValidity, l.uid)
        withContext(Dispatchers.IO) { if (file.exists()) runCatching { file.readBytes() }.getOrNull() else null }?.let { return it }
        return try {
            download(l.acc, l.box, l.folder, l.uid, file, background = false)
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            throw FriendlyErrors.wrap(e, strings)
        }
    }

    /**
     * One download per message at a time. A message the user opens while it waits as background work
     * is fetched again in the user's turn, rather than behind the rest of the prefetch.
     */
    private suspend fun download(acc: Account, box: CachedBox, folder: String, uid: Long, file: File, background: Boolean): ByteArray {
        val key = file.path
        downloads[key]?.let { running -> if (background || !running.background) return running.raw.await() }
        lateinit var entry: Download
        val raw = scope.async(engine, start = CoroutineStart.LAZY) {
            try {
                val bytes = session(acc).fetchSource(folder, uid, background)
                // Not kept for an account removed meanwhile, or a folder whose uids were reassigned.
                val now = caches.value[acc.id]?.boxes?.get(folder)
                if (accountOrNull(acc.id) != null && now != null && now.uidValidity == box.uidValidity) {
                    withContext(Dispatchers.IO) {
                        runCatching {
                            file.parentFile?.mkdirs()
                            val tmp = File(file.parentFile, "${file.name}.${Ids.rand(8)}.tmp")
                            tmp.writeBytes(bytes)
                            if (!tmp.renameTo(file)) tmp.delete()
                        }
                    }
                }
                bytes
            } finally {
                if (downloads[key] === entry) downloads.remove(key)
            }
        }
        entry = Download(background, raw)
        downloads[key] = entry
        raw.start()
        return raw.await()
    }

    /**
     * Downloads the newest inbox messages ahead of time, so opening one reads it from disk. Bodies of
     * messages that left the synced folders go first (engine.js prefetch, pruneBodies).
     */
    private suspend fun prefetch(accountId: String) {
        if (closed || !prefetching.add(accountId)) return
        try {
            val acc = accountOrNull(accountId) ?: return
            val cache = caches.value[accountId] ?: return
            pruneBodies(accountId, cache)
            val inbox = cache.folderByRole(FolderRole.INBOX) ?: return
            val box = cache.boxes[inbox.path] ?: return
            for (m in box.messages.sortedByDescending { it.date }.take(Limits.PREFETCH_COUNT)) {
                if (closed || accountOrNull(accountId) == null) return
                val file = bodyFile(accountId, inbox.path, box.uidValidity, m.uid)
                if (m.size > Limits.PREFETCH_MAX_BYTES || withContext(Dispatchers.IO) { file.exists() }) continue
                download(acc, box, inbox.path, m.uid, file, background = true)
            }
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            // Offline, signed out or the message is gone: the next sync tries again.
        } finally {
            prefetching.remove(accountId)
        }
    }

    private suspend fun pruneBodies(accountId: String, cache: AccountCache) {
        val keep = HashSet<String>()
        for ((folder, box) in cache.boxes) for (m in box.messages) keep += bodyFile(accountId, folder, box.uidValidity, m.uid).name
        withContext(Dispatchers.IO) {
            bodiesDir(accountId).listFiles()?.forEach { if (it.name.endsWith(".eml") && it.name !in keep) it.delete() }
        }
    }

    private suspend fun parsed(l: Located): ParsedMail {
        val key = "${l.id}|${l.box.uidValidity}"
        parsedCache[key]?.let { return it }
        val raw = rawSourceNow(l)
        val parsed = withContext(Dispatchers.Default) {
            try {
                MimeParser.parse(raw)
            } catch (_: StackOverflowError) {
                // A crafted message the mail library still parses recursively: an error, not a crash.
                throw MailException(strings.get(R.string.engine_errors_message_too_deep), "too-deep")
            }
        }
        parsedCache[key] = parsed
        return parsed
    }

    private fun attachmentName(filename: String?, index: Int) = filename ?: strings.get(R.string.engine_attachments_default_numbered, index + 1)

    override suspend fun fullMessage(id: String): FullMessage = on {
        val l = locate(id)
        val p = parsed(l)
        val summary = publicOf(l)
        withContext(Dispatchers.Default) {
            val html = p.html?.let { MailHtml.sanitize(it, p.inlineImages()) } ?: MailText.textToHtml(p.text.orEmpty())
            FullMessage(
                summary = summary,
                from = p.from.firstOrNull() ?: summary.from,
                to = p.to,
                cc = p.cc,
                bcc = p.bcc,
                replyTo = p.replyTo,
                subject = p.subject?.takeIf { it.isNotEmpty() } ?: summary.subject,
                date = p.date ?: summary.date,
                messageId = p.messageId,
                inReplyTo = p.inReplyTo,
                references = p.references,
                html = html,
                isHtml = p.html != null,
                text = p.text?.takeIf { it.isNotEmpty() } ?: MailText.htmlToPlain(p.html.orEmpty()),
                unsubscribe = p.unsubscribe,
                attachments = p.attachments.filter { !it.related }.map {
                    AttachmentInfo(it.index, attachmentName(it.filename, it.index), it.contentType, it.size, false, it.contentId)
                },
            )
        }
    }

    override suspend fun attachment(id: String, index: Int): AttachmentContent = on {
        val a = parsed(locate(id)).attachments.getOrNull(index) ?: strings.fail(R.string.engine_errors_attachment_missing, code = "missing")
        AttachmentContent(attachmentName(a.filename, a.index), a.contentType, a.charset, a.bytes)
    }

    override suspend fun attachmentFile(id: String, index: Int): File {
        val content = attachment(id, index)
        return withContext(Dispatchers.IO) {
            val digest = MessageDigest.getInstance("SHA-1").digest(id.toByteArray(Charsets.UTF_8)).joinToString("") { "%02x".format(it) }
            // One folder per attachment: two parts of a message may well share a name ("invoice.pdf").
            val folder = File(File(attachmentsDir, digest), index.toString()).apply { mkdirs() }
            File(folder, safeName(content.filename, index)).apply { writeBytes(content.bytes) }
        }
    }

    // ---------- changes ----------

    private suspend fun <T> withPending(accountId: String, folder: String, uids: List<Long>, patch: PendingPatch, op: suspend () -> T): T {
        val keys = uids.map { CacheOps.pendingKey(accountId, folder, it) }
        for (k in keys) pending[k] = pending[k]?.merge(patch) ?: patch
        try {
            return op()
        } finally {
            keys.forEach { pending.remove(it) }
        }
    }

    override suspend fun setFlags(ids: List<String>, unread: Boolean?, starred: Boolean?, answered: Boolean?): Unit = change {
        setFlagsNow(ids, unread, starred, answered)?.let { throw it }
    }

    /** Returns the first failure instead of throwing, so callers that ignore failures can. */
    private suspend fun setFlagsNow(ids: List<String>, unread: Boolean?, starred: Boolean?, answered: Boolean?): MailException? {
        val patch = PendingPatch(unread, starred, answered)
        var error: MailException? = null
        val located = ids.mapNotNull { id ->
            locateOrNull(id) ?: run {
                if (error == null) error = MailException(strings.get(R.string.engine_errors_message_missing), "missing")
                null
            }
        }
        for ((key, group) in located.groupBy { it.acc.id to it.folder }) {
            val (accountId, folder) = key
            // Flag changes of one folder take turns from snapshot to rollback: a failed change puts back the values
            // from before it, which must not overwrite a later change that is still on its way.
            flagLocks.getOrPut("$accountId\n$folder") { Mutex() }.withLock {
                val before = caches.value[accountId]?.boxes?.get(folder)?.messages.orEmpty()
                    .filter { m -> group.any { it.uid == m.uid } }.associateBy { it.uid }
                val uids = before.keys.toList()
                if (uids.isEmpty()) return@withLock
                flagGroup(group.first().acc, folder, uids, before, patch)?.let { if (error == null) error = it }
            }
        }
        return error
    }

    /** One folder's flag change: optimistic in the cache, then the server; a refusal puts the earlier values back. */
    private suspend fun flagGroup(acc: Account, folder: String, uids: List<Long>, before: Map<Long, CachedMessage>, patch: PendingPatch): MailException? {
        val (unread, starred, answered) = Triple(patch.unread, patch.starred, patch.answered)
        mutate(acc.id) { c -> uids.fold(c) { a, uid -> CacheOps.updateMessage(a, folder, uid, patch::applyTo) } }
        try {
            withPending(acc.id, folder, uids, patch) { session(acc).setFlags(folder, uids, unread, starred, answered) }
            return null
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            // Back to each message's own earlier value, on whatever the cache holds now (a sync may have replaced it),
            // and only where the value is still the one set here.
            mutate(acc.id) { c ->
                uids.fold(c) { a, uid ->
                    CacheOps.updateMessage(a, folder, uid) { m ->
                        val old = before.getValue(uid)
                        m.copy(
                            unread = if (unread != null && m.unread == unread) old.unread else m.unread,
                            starred = if (starred != null && m.starred == starred) old.starred else m.starred,
                            answered = if (answered != null && m.answered == answered) old.answered else m.answered,
                        )
                    }
                }
            }
            return FriendlyErrors.wrap(e, strings)
        }
    }

    override suspend fun markAllRead(view: MailView): Int = change {
        val ids = MailViews.list(live(), view, "", SortOrder.DATE_DESC).filter { it.unread }.map { it.id }
        setFlagsNow(ids, unread = false, starred = null, answered = null)
        ids.size
    }

    /** Moves located messages of one account and folder; desktop's move per message, as one server command. */
    private suspend fun moveGroup(acc: Account, folder: String, items: List<Located>, destination: String, moved: MutableList<Moved>, failed: MutableList<Failed>) {
        if (folder == destination) {
            items.forEach { moved += Moved(it.id, it.id, undoable = false) }
            return
        }
        val uids = items.map { it.uid }
        mutate(acc.id) { c -> uids.fold(c) { a, uid -> CacheOps.removeMessage(a, folder, uid) } }
        val outcome = try {
            withPending(acc.id, folder, uids, PendingPatch(removed = true)) { session(acc).move(folder, uids, destination) }
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            mutate(acc.id) { c -> items.fold(c) { a, l -> CacheOps.addMessage(a, folder, l.msg) } }
            val text = friendly(e)
            items.forEach { failed += Failed(it.id, text) }
            return
        }
        val now = clock.now()
        // Purge expired undo records on each insert.
        undoable.entries.removeIf { now - it.value.at > Limits.UNDO_MS }
        for (l in items) {
            val newUid = outcome.uids[l.uid]
            // Servers without UIDPLUS don't report the new uid; such a move can't be undone.
            val canUndo = newUid != null && outcome.uidValidity != null
            if (canUndo) undoable[l.id] = UndoRecord(acc.id, folder, destination, newUid, outcome.uidValidity, l.msg, now)
            moved += Moved(l.id, newUid?.let { MessageIds.encode(acc.id, destination, it) }, canUndo)
            mutate(acc.id) { c ->
                // A cached destination gets the message under its new uid right away, so the new id works at once
                // and the follow-up sync doesn't take it for new mail.
                val box = c.boxes[destination]
                val placed = if (newUid != null && box != null && box.uidValidity == outcome.uidValidity) CacheOps.addMessage(c, destination, l.msg.copy(uid = newUid)) else c
                CacheOps.noteMove(placed, l.msg.messageId, destination, folder)
            }
        }
        if (caches.value[acc.id]?.boxes?.containsKey(destination) == true) resyncLater(acc, destination)
    }

    private suspend fun moveAll(ids: List<String>, target: (Located) -> String?, noTarget: () -> String): MoveResult {
        val moved = mutableListOf<Moved>()
        val failed = mutableListOf<Failed>()
        val located = ids.mapNotNull { id -> locateOrNull(id) ?: null.also { failed += Failed(id, strings.get(R.string.engine_errors_message_missing)) } }
        var folderUsed: String? = null
        for ((key, group) in located.groupBy { it.acc.id to it.folder }) {
            val destination = target(group.first())
            if (destination == null) {
                val text = noTarget()
                group.forEach { failed += Failed(it.id, text) }
                continue
            }
            folderUsed = folderUsed ?: destination
            moveGroup(group.first().acc, key.second, group, destination, moved, failed)
        }
        return MoveResult(moved, failed, folderUsed)
    }

    override suspend fun move(ids: List<String>, targetPath: String): MoveResult = change {
        moveAll(ids, { targetPath }, { "" })
    }

    override suspend fun archive(ids: List<String>): MoveResult = change {
        moveAll(ids, { caches.value[it.acc.id]?.archiveFolder()?.path }, { strings.get(R.string.engine_errors_folder_no_archive) })
    }

    /**
     * Moves to Trash, or deletes for good when the message is in Trash already or the account has none.
     * Deleting for good needs [force]; without it nothing changes and the answer asks for confirmation.
     */
    override suspend fun delete(ids: List<String>, force: Boolean): DeleteResult = change {
        val located = ids.mapNotNull { locateOrNull(it) }
        val missing = ids.filter { id -> located.none { it.id == id } }.map { Failed(it, strings.get(R.string.engine_errors_message_missing)) }
        fun trashOf(l: Located) = caches.value[l.acc.id]?.folderByRole(FolderRole.TRASH)?.path
        val (toTrash, forever) = located.partition { l -> trashOf(l)?.let { it != l.folder } == true }
        if (forever.isNotEmpty() && !force) return@change DeleteResult(DeleteOutcome.NEEDS_CONFIRM, MoveResult(failed = missing))
        val moved = mutableListOf<Moved>()
        val failed = missing.toMutableList()
        var trashFolder: String? = null
        for ((key, group) in toTrash.groupBy { it.acc.id to it.folder }) {
            val trash = trashOf(group.first())!!
            trashFolder = trashFolder ?: trash
            moveGroup(group.first().acc, key.second, group, trash, moved, failed)
        }
        var deleted = 0
        for ((key, group) in forever.groupBy { it.acc.id to it.folder }) {
            val (accountId, folder) = key
            val uids = group.map { it.uid }
            mutate(accountId) { c -> uids.fold(c) { a, uid -> CacheOps.removeMessage(a, folder, uid) } }
            try {
                withPending(accountId, folder, uids, PendingPatch(removed = true)) { session(group.first().acc).deleteForever(folder, uids) }
                deleted += group.size
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                mutate(accountId) { c -> group.fold(c) { a, l -> CacheOps.addMessage(a, folder, l.msg) } }
                val text = friendly(e)
                group.forEach { failed += Failed(it.id, text) }
            }
        }
        DeleteResult(if (deleted > 0) DeleteOutcome.DELETED else DeleteOutcome.TRASHED, MoveResult(moved, failed, trashFolder))
    }

    override suspend fun emptyFolder(accountId: String, path: String): Int = change {
        val ids = MailViews.list(live(), MailView.folder(accountId, path), "", SortOrder.DATE_DESC).map { it.id }
        if (ids.isNotEmpty()) delete(ids, force = true)
        ids.size
    }

    override fun canUndo(oldId: String): Boolean = undoable[oldId]?.let { clock.now() - it.at <= Limits.UNDO_MS } == true

    /** Moves messages back to where they were before move, delete or archive (engine.js undoMove). */
    override suspend fun undoMove(oldIds: List<String>): UndoResult = change {
        val restored = mutableListOf<String>()
        val failed = mutableListOf<Failed>()
        val records = oldIds.mapNotNull { id ->
            val rec = undoable.remove(id)
            if (rec == null || clock.now() - rec.at > Limits.UNDO_MS || accountOrNull(rec.accountId) == null) {
                failed += Failed(id, strings.get(R.string.engine_errors_undo_expired))
                null
            } else {
                id to rec
            }
        }
        for ((_, group) in records.groupBy { (_, r) -> listOf(r.accountId, r.destination, r.folder, r.uidValidity) }) {
            val first = group.first().second
            val acc = account(first.accountId)
            val uids = group.map { it.second.uid }
            mutate(acc.id) { c -> uids.fold(c) { a, uid -> CacheOps.removeMessage(a, first.destination, uid) } }
            val outcome = try {
                withPending(acc.id, first.destination, uids, PendingPatch(removed = true)) {
                    session(acc).move(first.destination, uids, first.folder, expectUidValidity = first.uidValidity)
                }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                if (caches.value[acc.id]?.boxes?.containsKey(first.destination) == true) resyncLater(acc, first.destination)
                val text = if (e is UidValidityChanged) strings.get(R.string.engine_errors_undo_rebuilt) else friendly(e)
                group.forEach { (id, _) -> failed += Failed(id, text) }
                continue
            }
            for ((_, rec) in group) {
                val uid = outcome.uids[rec.uid]
                // Put it back right away; with the new uid cached, the follow-up sync doesn't report it as new mail.
                mutate(acc.id) { c ->
                    val back = if (uid != null) CacheOps.addMessage(c, rec.folder, rec.msg.copy(uid = uid)) else c
                    CacheOps.noteMove(back, rec.msg.messageId, rec.folder, rec.destination)
                }
                if (uid != null) restored += MessageIds.encode(acc.id, rec.folder, uid)
            }
            if (caches.value[acc.id]?.boxes?.containsKey(first.folder) == true) resyncLater(acc, first.folder)
        }
        UndoResult(restored, failed)
    }

    override suspend fun createFolder(accountId: String, name: String): String = change {
        val clean = name.trim()
        if (clean.isEmpty()) strings.fail(R.string.engine_errors_folder_name_required, code = "name")
        if (clean.length > 100 || Regex("[\\\\/%*]").containsMatchIn(clean)) strings.fail(R.string.engine_errors_folder_invalid_name, code = "name")
        val acc = account(accountId)
        val cache = caches.value[accountId] ?: AccountCache()
        if (cache.folders.any { it.name.equals(clean, ignoreCase = true) }) strings.fail(R.string.engine_errors_folder_duplicate, code = "exists")
        try {
            val created = session(acc).createFolder(clean)
            val listed = session(acc).listFolders()
            mutate(accountId) { it.copy(folders = listed) }
            created
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            throw FriendlyErrors.wrap(e, strings)
        }
    }

    override suspend fun toggleVip(address: String): Boolean = change {
        val a = address.trim().lowercase()
        val vips = _prefs.value.vips.toMutableSet()
        val now = if (a in vips) { vips.remove(a); false } else { vips.add(a); true }
        savePrefs(_prefs.value.copy(vips = vips.toList()))
        now
    }

    override suspend fun markSpam(ids: List<String>): MoveResult = change {
        val located = ids.mapNotNull { locateOrNull(it) }
        val spam = (_prefs.value.spam + located.map { it.msg.from.address.trim().lowercase() }.filter { it.isNotEmpty() }).distinct()
        savePrefs(_prefs.value.copy(spam = spam))
        val withJunk = located.filter { caches.value[it.acc.id]?.folderByRole(FolderRole.JUNK) != null }.map { it.id }
        if (withJunk.isEmpty()) MoveResult() else moveAll(withJunk, { caches.value[it.acc.id]?.folderByRole(FolderRole.JUNK)?.path }, { "" })
    }

    override suspend fun updatePrefs(transform: (MailPrefs) -> MailPrefs): MailPrefs = change {
        val next = transform(_prefs.value)
        val html = try {
            SignatureHtml.clean(next.signatureHtml)
        } catch (e: SignatureHtml.TooLarge) {
            strings.fail(R.string.engine_settings_signature_too_large, code = "signature")
        }
        val cleaned = next.copy(
            vips = next.vips.map { it.trim().lowercase() }.filter { it.isNotEmpty() }.distinct(),
            spam = next.spam.map { it.trim().lowercase() }.filter { it.isNotEmpty() }.distinct(),
            signatureHtml = html,
            signature = if (html.isNotEmpty()) SignatureHtml.text(html) else next.signature,
        )
        savePrefs(cleaned)
        cleaned
    }

    private suspend fun savePrefs(prefs: MailPrefs) {
        _prefs.value = prefs
        withContext(Dispatchers.IO) { JsonFiles.write(prefsFile, MailPrefs.serializer(), prefs) }
    }

    // ---------- sending ----------

    private val emailRe = Regex("^[^@\\s]+@[^@\\s]+\\.[^@\\s]+$")

    private suspend fun buildMail(acc: Account, message: OutgoingMessage, keepBcc: Boolean): ComposedMail {
        val identity = MailViews.identity(acc, message.from.address.trim().ifEmpty { null })
            ?: strings.fail(R.string.engine_errors_send_identity, message.from.address, code = "identity")
        val body = ComposeBody.build(message.text, if (message.signature) signatureFor(acc.id) else null, message.quote, message.html)
        val (html, inline) = ComposeBody.inlineDataImages(body.html)
        val files = message.attachments.map { a ->
            if (a.sourceMessageId != null && a.sourceIndex != null) {
                val content = attachmentNow(a.sourceMessageId, a.sourceIndex)
                MailAttachment(a.filename.ifEmpty { content.filename }, content.contentType, content.bytes)
            } else {
                val bytes = withContext(Dispatchers.IO) { File(a.path).readBytes() }
                MailAttachment(a.filename.ifEmpty { File(a.path).name }, a.contentType, bytes)
            }
        }
        fun clean(list: List<Address>) = list.map { it.copy(address = it.address.trim()) }.filter { it.address.isNotEmpty() }
        val domain = identity.address.substringAfter('@', "").ifEmpty { "rukoo.invalid" }
        return ComposedMail(
            from = Address(identity.name.ifEmpty { acc.name }, identity.address),
            to = clean(message.to),
            cc = clean(message.cc),
            bcc = clean(message.bcc),
            subject = message.subject,
            text = body.text,
            html = html,
            inReplyTo = message.inReplyTo?.takeIf { it.isNotBlank() },
            references = message.references.filter { it.isNotBlank() },
            date = clock.now(),
            // A Message-ID of our own finds a stored draft again on servers that don't report its uid.
            messageId = "<${Ids.uuid()}@$domain>",
            attachments = files + inline,
            keepBcc = keepBcc,
        )
    }

    private suspend fun attachmentNow(id: String, index: Int): AttachmentContent {
        val a = parsed(locate(id)).attachments.getOrNull(index) ?: strings.fail(R.string.engine_errors_attachment_missing, code = "missing")
        return AttachmentContent(attachmentName(a.filename, a.index), a.contentType, a.charset, a.bytes)
    }

    override suspend fun send(message: OutgoingMessage) = change {
        val acc = account(message.accountId)
        val mail = buildMail(acc, message, keepBcc = false)
        if (mail.recipients.isEmpty()) strings.fail(R.string.engine_errors_send_no_recipients, code = "recipients")
        mail.recipients.firstOrNull { !emailRe.matches(it.address) }?.let { strings.fail(R.string.engine_common_errors_invalid_email_value, it.address, code = "recipients") }
        val raw = try {
            session(acc).send(mail)
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            throw FriendlyErrors.wrap(e, strings)
        }
        // New settings close the session sending started on, so the Sent copy goes through the account's
        // session as it is now; an account removed meanwhile keeps no copy.
        accountOrNull(acc.id)?.let { now ->
            val session = session(now)
            val sent = caches.value[now.id]?.folderByRole(FolderRole.SENT)
            if (sent != null && !session.savesSentCopy()) {
                runCatching { session.append(sent.path, raw, listOf("\\Seen")) }.onFailure { AppLog.w(TAG, "could not file the Sent copy", it) }
            }
        }
        message.draftId?.let { id -> runCatching { discardDraftNow(id) } }
        if (message.sourceMessageId != null && (message.replyKind == ReplyKind.REPLY || message.replyKind == ReplyKind.REPLY_ALL)) {
            // The original may have moved meanwhile; that is fine.
            setFlagsNow(listOf(message.sourceMessageId), unread = null, starred = null, answered = true)
        }
        syncAccount(acc.id)
        Unit
    }

    override suspend fun saveDraft(message: OutgoingMessage): String? = change {
        val acc = account(message.accountId)
        val drafts = caches.value[acc.id]?.folderByRole(FolderRole.DRAFTS) ?: strings.fail(R.string.engine_errors_folder_no_drafts, code = "drafts")
        val mail = buildMail(acc, message, keepBcc = true)
        try {
            val raw = withContext(Dispatchers.Default) { MimeComposer.compose(mail) }
            val uid = session(acc).append(drafts.path, raw, listOf("\\Seen", "\\Draft"))
            syncFolderInto(acc, drafts.path)
            persistCache(acc.id)
            val stored = uid ?: caches.value[acc.id]?.let { CacheOps.uidByMessageId(it, drafts.path, mail.messageId) }
            if (stored != null) {
                // Kept as the new copy's body, so reopening it or reading its attachments back needs no download.
                caches.value[acc.id]?.boxes?.get(drafts.path)?.let { box ->
                    withContext(Dispatchers.IO) {
                        runCatching { bodyFile(acc.id, drafts.path, box.uidValidity, stored).apply { parentFile?.mkdirs() }.writeBytes(raw) }
                    }
                }
                // The old copy goes only once the new one is found: a server that doesn't say where it put it keeps both.
                message.draftId?.let { id -> runCatching { discardDraftNow(id) } }
            }
            stored?.let { MessageIds.encode(acc.id, drafts.path, it) }
        } catch (e: CancellationException) {
            throw e
        } catch (e: MailException) {
            throw e
        } catch (e: Exception) {
            throw FriendlyErrors.wrap(e, strings)
        }
    }

    override suspend fun discardDraft(id: String) = change { discardDraftNow(id) }

    private suspend fun discardDraftNow(id: String) {
        val l = locate(id)
        mutate(l.acc.id) { CacheOps.removeMessage(it, l.folder, l.uid) }
        try {
            session(l.acc).deleteForever(l.folder, listOf(l.uid))
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            throw FriendlyErrors.wrap(e, strings)
        }
    }

    override suspend fun replyFields(id: String, kind: ReplyKind): ComposerFields {
        val m = fullMessage(id)
        return on {
            val acc = account(m.accountId)
            val labels = QuoteLabels(
                strings.get(R.string.engine_composer_quote_marker),
                strings.get(R.string.engine_composer_quote_from, "%1\$s"),
                strings.get(R.string.engine_composer_quote_date, "%1\$s"),
                strings.get(R.string.engine_composer_quote_to, "%1\$s"),
                strings.get(R.string.engine_composer_quote_subject, "%1\$s"),
            )
            ReplyBuilder.build(kind, m, acc, MailViews.identities(acc), labels, zone(), locale())
        }
    }

    /**
     * Unsubscribes from a list (main.js unsubscribe): one-click POST when the sender offers it and its
     * endpoint answers 2xx, else the web page, else a filled-in email for the user to send.
     */
    override suspend fun unsubscribe(id: String): UnsubscribeResult {
        val m = fullMessage(id)
        val u = m.unsubscribe ?: return UnsubscribeResult.Unavailable
        val acc = on { account(m.accountId) }
        if (acc.type == AccountType.DEMO) return UnsubscribeResult.Done
        if (u.oneClick && u.url != null && safeHttp.oneClickUnsubscribe(u.url)) return UnsubscribeResult.Done
        if (u.url != null) return UnsubscribeResult.OpenPage(u.url)
        val fields = Mailto.parse(u.mail)
        return UnsubscribeResult.SendEmail(
            to = fields.to.joinToString(", ") { it.address },
            subject = fields.subject.ifEmpty { strings.get(R.string.engine_reader_unsubscribe_subject) },
            body = fields.body,
        )
    }

    // ---------- account management ----------

    private fun color(): String = Account.COLORS[_accounts.value.size % Account.COLORS.size]

    /** Verifies the sign-in, then saves the account and starts its first sync (engine.js finishAdd). */
    private suspend fun finishAdd(acc: Account, secret: String?): Account {
        if (_accounts.value.any { it.email.equals(acc.email, ignoreCase = true) }) strings.fail(R.string.engine_errors_account_duplicate, code = "duplicate")
        val session = factory.create(acc, credentials(acc, secret))
        try {
            session.verify()
        } catch (e: Exception) {
            tokens.remove(acc.id)
            runCatching { session.close() }
            if (e is CancellationException) throw e
            throw FriendlyErrors.wrap(e, strings)
        }
        if (acc.type == AccountType.IMAP && secret != null) secrets = secrets + (acc.id to cipher.encrypt(secret))
        sessions[acc.id] = session
        caches.update { it + (acc.id to AccountCache()) }
        _accounts.update { it + acc }
        persistAccounts()
        persistCache(acc.id)
        runCatching { onAccountAdded(acc.id) }
        onAccountsChanged(_accounts.value)
        syncAccount(acc.id)
        return acc
    }

    override suspend fun addImapAccount(input: ImapAccountInput): Account = change {
        val email = input.email.trim()
        if (!emailRe.matches(email)) strings.fail(R.string.engine_errors_account_invalid_email, code = "email")
        if (input.password.isEmpty()) strings.fail(R.string.engine_errors_account_password, code = "password")
        val (imapDefault, smtpDefault) = MailProviders.serverDefaults(input.provider, email)
        val imap = (if (input.imap.host.isBlank()) imapDefault else input.imap).let { it.copy(host = it.host.trim(), user = it.user.trim().ifEmpty { email }) }
        val smtp = (if (input.smtp.host.isBlank()) smtpDefault else input.smtp).let { it.copy(host = it.host.trim(), user = it.user.trim().ifEmpty { email }) }
        val acc = Account(
            id = Ids.uuid(),
            type = AccountType.IMAP,
            provider = input.provider,
            auth = AuthKind.PASSWORD,
            email = email,
            name = input.name.trim().ifEmpty { email.substringBefore('@') },
            label = email,
            color = color(),
            imap = imap,
            smtp = smtp,
            createdAt = clock.now(),
        )
        finishAdd(acc, input.password)
    }

    override suspend fun addGoogleAccount(tokens: GoogleTokens): Account = change {
        val email = tokens.email.lowercase()
        val (imap, smtp) = MailProviders.serverDefaults("google", email)
        val acc = Account(
            id = Ids.uuid(),
            type = AccountType.IMAP,
            provider = "google",
            auth = AuthKind.OAUTH2,
            email = email,
            name = tokens.name.ifEmpty { email.substringBefore('@') },
            label = email,
            color = color(),
            imap = imap.copy(user = email),
            smtp = smtp.copy(user = email),
            scopes = tokens.scopes.distinct().sorted(),
            createdAt = clock.now(),
        )
        if (tokens.accessToken != null && tokens.expiresAt != null) this.tokens[acc.id] = TokenEntry(tokens.accessToken, tokens.expiresAt)
        finishAdd(acc, tokens.refreshToken)
    }

    override suspend fun googleClient(): GoogleClient? = googleAuth?.client()

    override suspend fun setGoogleClient(client: GoogleClient) {
        val google = googleAuth ?: strings.fail(R.string.engine_errors_google_unavailable, code = MailEngine.UNSUPPORTED)
        withContext(Dispatchers.IO) { google.setClient(client) }
    }

    /**
     * The browser part runs in the caller's context, not on the engine's thread, which must not wait
     * minutes for the user; only saving the account goes through it.
     */
    override suspend fun signInWithGoogle(openUrl: (String) -> Unit, reauthAccountId: String?): Account {
        val google = googleAuth ?: strings.fail(R.string.engine_errors_google_unavailable, code = MailEngine.UNSUPPORTED)
        val hint = reauthAccountId?.let { id -> on { account(id).email } }
        val tokens = google.signIn(openUrl, hint)
        return if (reauthAccountId != null) reauthorizeGoogle(reauthAccountId, tokens) else addGoogleAccount(tokens)
    }

    /** Signs a Google account in again with new [tokens]; the email must be the account's. */
    private suspend fun reauthorizeGoogle(accountId: String, tokens: GoogleTokens): Account = change {
        val acc = account(accountId)
        if (!acc.email.equals(tokens.email, ignoreCase = true)) {
            strings.fail(R.string.engine_errors_google_wrong_account, tokens.email, acc.email, code = "wrong_account")
        }
        val next = acc.copy(auth = AuthKind.OAUTH2, scopes = tokens.scopes.distinct().sorted())
        secrets = secrets + (acc.id to cipher.encrypt(tokens.refreshToken))
        this.tokens[acc.id] = TokenEntry(tokens.accessToken, tokens.expiresAt ?: 0)
        replaceAccount(next)
        sessions.remove(acc.id)?.let { runCatching { it.close() } }
        persistAccounts()
        syncAccount(acc.id)
        next
    }

    override suspend fun addDemoAccount(): Account = change {
        _accounts.value.firstOrNull { it.type == AccountType.DEMO }?.let { return@change it }
        val acc = Account(
            id = Ids.uuid(),
            type = AccountType.DEMO,
            provider = "demo",
            auth = AuthKind.NONE,
            email = Account.DEMO_EMAIL,
            name = strings.get(R.string.engine_setup_demo_name),
            label = Account.DEMO_EMAIL,
            color = color(),
            createdAt = clock.now(),
        )
        finishAdd(acc, null)
    }

    /**
     * Accounts from desktop Rukoo's export (spec 3 §7.10), saved without signing in again: they work on
     * the computer, and the first sync reports a problem like any other. Accounts already here (by
     * email) and accounts whose secret the computer couldn't decrypt are skipped.
     */
    override suspend fun importAccounts(imported: List<ImportedAccount>, googleClient: GoogleClient?): List<Account> = change {
        googleClient?.takeIf { it.clientId.isNotBlank() && it.clientSecret.isNotBlank() }?.let { googleAuth?.setClient(it) }
        val added = mutableListOf<Account>()
        for (i in imported) {
            val email = i.email.trim()
            if (email.isEmpty() || _accounts.value.any { it.email.equals(email, ignoreCase = true) }) continue
            val demo = i.type == AccountType.DEMO
            val auth = if (demo) AuthKind.NONE else if (i.auth == AuthKind.OAUTH2) AuthKind.OAUTH2 else AuthKind.PASSWORD
            val secret = if (auth == AuthKind.OAUTH2) i.refreshToken else i.password
            if (!demo && (i.secretError != null || secret.isNullOrEmpty())) continue
            val id = i.id.takeIf { it.isNotBlank() && _accounts.value.none { a -> a.id == it } } ?: Ids.uuid()
            val (imapDefault, smtpDefault) = MailProviders.serverDefaults(if (i.provider in MailProviders.all.map { it.id }) i.provider else "other", email)
            val acc = Account(
                id = id,
                type = if (demo) AccountType.DEMO else AccountType.IMAP,
                provider = if (demo) "demo" else i.provider,
                auth = auth,
                email = if (demo) Account.DEMO_EMAIL else email,
                name = i.name.ifEmpty { email.substringBefore('@') },
                label = i.label.ifEmpty { email },
                color = i.color.ifEmpty { color() },
                signature = i.signature,
                signatureHtml = i.signatureHtml?.let(SignatureHtml::reclean)?.ifEmpty { null },
                imap = if (demo) null else (i.imap ?: imapDefault).let { it.copy(user = it.user.ifEmpty { email }) },
                smtp = if (demo) null else (i.smtp ?: smtpDefault).let { it.copy(user = it.user.ifEmpty { email }) },
                scopes = i.scopes.distinct().sorted(),
                aliases = i.aliases.map { Alias(it.address.lowercase(), it.name) }.filter { !it.address.equals(email, true) },
                defaultFrom = i.defaultFrom?.lowercase(),
                createdAt = i.createdAt.takeIf { it > 0 } ?: clock.now(),
            )
            if (!demo) secrets = secrets + (id to cipher.encrypt(secret!!))
            caches.update { it + (id to AccountCache()) }
            _accounts.update { it + acc }
            added += acc
        }
        if (added.isNotEmpty()) {
            persistAccounts()
            added.forEach { persistCache(it.id) }
            added.forEach { runCatching { onAccountAdded(it.id) } }
            onAccountsChanged(_accounts.value)
            added.forEach { syncAccount(it.id) }
        }
        added
    }

    override suspend fun updateAccount(id: String, patch: AccountPatch): Account = change {
        val acc = account(id)
        var next = acc
        patch.name?.let { next = next.copy(name = it) }
        patch.label?.let { next = next.copy(label = it) }
        patch.color?.let { if (it.isNotBlank()) next = next.copy(color = it) }
        if (patch.signature != null || patch.signatureHtml != null) {
            // null resets the account to the general signature; cleaned first, so one too large changes nothing.
            val reset = (patch.signature != null && patch.signature.value == null) || (patch.signatureHtml != null && patch.signatureHtml.value == null)
            val html = patch.signatureHtml?.value
            next = when {
                reset -> next.copy(signature = null, signatureHtml = null)
                html != null -> {
                    val clean = try {
                        SignatureHtml.clean(html)
                    } catch (e: SignatureHtml.TooLarge) {
                        strings.fail(R.string.engine_settings_signature_too_large, code = "signature")
                    }
                    next.copy(signature = SignatureHtml.text(clean), signatureHtml = clean.ifEmpty { null })
                }
                else -> next.copy(signature = patch.signature?.value.orEmpty(), signatureHtml = null)
            }
        }
        patch.aliases?.let { list ->
            // The whole list is checked before anything changes, so a typo can't wipe existing aliases.
            val seen = mutableSetOf(acc.email.lowercase())
            val aliases = list.mapNotNull { a ->
                val address = a.address.trim().lowercase()
                if (!emailRe.matches(address)) strings.fail(R.string.engine_common_errors_invalid_email_value, a.address, code = "alias")
                if (!seen.add(address)) null else Alias(address, a.name)
            }
            next = next.copy(aliases = aliases)
            if (next.defaultFrom != null && MailViews.identity(next, next.defaultFrom) == null) next = next.copy(defaultFrom = null)
        }
        patch.defaultFrom?.let { change ->
            val value = change.value?.trim()?.lowercase()?.ifEmpty { null }
            if (value != null && MailViews.identity(next, value) == null) strings.fail(R.string.engine_errors_account_identity, code = "identity")
            next = next.copy(defaultFrom = value)
        }
        if (patch.password != null && acc.type == AccountType.IMAP && acc.auth != AuthKind.OAUTH2) {
            val probe = factory.create(next, Credentials.Password(patch.password))
            try {
                probe.verify()
            } catch (e: Exception) {
                if (e is CancellationException) throw e
                throw FriendlyErrors.wrap(e, strings)
            } finally {
                runCatching { probe.close() }
            }
            secrets = secrets + (acc.id to cipher.encrypt(patch.password))
            sessions.remove(acc.id)?.let { runCatching { it.close() } }
        }
        replaceAccount(next)
        persistAccounts()
        next
    }

    override suspend fun removeAccount(id: String) = change {
        val acc = account(id)
        // Off the list before its connection closes, so a prefetch going on meanwhile doesn't open a new one.
        _accounts.update { list -> list.filter { it.id != id } }
        sessions.remove(id)?.let { runCatching { it.close() } }
        caches.update { it - id }
        saveJobs[id]?.let { if (!it.writing) it.job.cancel() }
        tokens.remove(id)
        secrets = secrets - id
        undoable.entries.removeIf { it.value.accountId == id }
        parsedCache.keys.removeIf { MessageIds.accountOf(it.substringBefore('|')) == id }
        // After a cache write already under way, so it can't bring the file back; later ones find no cache.
        writeLock(id).withLock {
            withContext(Dispatchers.IO) {
                cacheFile(id).delete()
                bodiesDir(id).deleteRecursively()
                if (acc.type == AccountType.DEMO) File(dir, DemoSession.FILE_NAME).delete()
            }
        }
        persistAccounts()
        runCatching { onAccountRemoved(id, _accounts.value.firstOrNull()?.id) }
        onAccountsChanged(_accounts.value)
        _events.tryEmit(MailEvent.AccountRemoved(id))
        Unit
    }

    /** Reads Gmail's verified "Send mail as" addresses (engine.js fetchGmailAliases). */
    private suspend fun fetchGmailAliasesNow(id: String): Account {
        val acc = account(id)
        if (acc.auth != AuthKind.OAUTH2) strings.fail(R.string.engine_errors_google_aliases_auth, code = "aliases")
        val token = accessToken(id)
        val request = Request.Builder().url("https://gmail.googleapis.com/gmail/v1/users/me/settings/sendAs").header("Authorization", "Bearer $token").build()
        val client = http.newBuilder().followRedirects(false).followSslRedirects(false).build()
        val (status, body) = try {
            withContext(Dispatchers.IO) { client.newCall(request).execute().use { it.code to it.body?.string().orEmpty() } }
        } catch (e: IOException) {
            strings.fail(R.string.engine_errors_google_unreachable, code = "network")
        }
        if (status !in 200..299) strings.fail(R.string.engine_errors_google_aliases, status.toString(), code = "aliases")
        val sendAs = runCatching { Json.parseToJsonElement(body).jsonObject["sendAs"]?.jsonArray.orEmpty().map { it.jsonObject } }.getOrDefault(emptyList())
        fun field(o: kotlinx.serialization.json.JsonObject, k: String) = o[k]?.jsonPrimitive?.content
        fun flag(o: kotlinx.serialization.json.JsonObject, k: String) = o[k]?.jsonPrimitive?.booleanOrNull == true
        val usable = sendAs.filter { flag(it, "isPrimary") || field(it, "verificationStatus") == "accepted" }
        val now = accountOrNull(id) ?: return acc
        var next = now.copy(
            aliases = usable.mapNotNull { s -> field(s, "sendAsEmail")?.lowercase()?.takeIf { !it.equals(now.email, true) }?.let { Alias(it, field(s, "displayName").orEmpty()) } },
        )
        // Only adopt Gmail's default when the user has not picked one in Rukoo.
        val gmailDefault = usable.firstOrNull { flag(it, "isDefault") }?.let { field(it, "sendAsEmail")?.lowercase() }
        if (next.defaultFrom == null && gmailDefault != null) next = next.copy(defaultFrom = gmailDefault)
        if (next.defaultFrom != null && MailViews.identity(next, next.defaultFrom) == null) next = next.copy(defaultFrom = null)
        next = next.copy(aliasesFetchedAt = clock.now())
        replaceAccount(next)
        persistAccounts()
        return next
    }

    /** Closes every session after writing pending caches; the engine takes no more work. */
    suspend fun close() {
        closed = true
        flush()
        on { sessions.values.forEach { runCatching { it.close() } }; sessions.clear() }
    }

    companion object {
        private const val TAG = "MailEngine"

        /** Flattens an attachment name to one safe file name: no folders, no bidi tricks (files.js safeName). */
        fun safeName(name: String, index: Int = 0): String {
            val flat = name.replace(Regex("[\\\\/]"), "_")
                .replace(Regex("[<>:\"|?*\\u0000-\\u001f\\u200e\\u200f\\u202a-\\u202e\\u2066-\\u2069]"), "_")
                .replace(Regex("^[.\\s]+|[.\\s]+$"), "")
            return flat.ifEmpty { "attachment-${index + 1}" }
        }
    }
}
