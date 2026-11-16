package nl.bvdm.rukoo.mail.session

import jakarta.mail.Flags
import jakarta.mail.Folder
import jakarta.mail.Session
import jakarta.mail.internet.InternetAddress
import jakarta.mail.internet.MimeMessage
import jakarta.mail.internet.MimeUtility
import java.io.ByteArrayInputStream
import java.util.Properties
import java.util.concurrent.Executors
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExecutorCoroutineDispatcher
import kotlinx.coroutines.asCoroutineDispatcher
import kotlinx.coroutines.withContext
import nl.bvdm.rukoo.R
import nl.bvdm.rukoo.categories.MailSignals
import nl.bvdm.rukoo.mail.MailException
import nl.bvdm.rukoo.mail.MailStrings
import nl.bvdm.rukoo.mail.mime.ComposedMail
import nl.bvdm.rukoo.mail.mime.MailText
import nl.bvdm.rukoo.mail.mime.MimeComposer
import nl.bvdm.rukoo.mail.mime.PartNode
import nl.bvdm.rukoo.mail.mime.Structure
import nl.bvdm.rukoo.mail.model.Account
import nl.bvdm.rukoo.mail.model.Address
import nl.bvdm.rukoo.mail.model.CachedMessage
import nl.bvdm.rukoo.mail.model.FolderRoles
import nl.bvdm.rukoo.mail.model.ServerConfig
import okhttp3.internal.tls.OkHostnameVerifier
import org.eclipse.angus.mail.iap.Argument
import org.eclipse.angus.mail.iap.Response
import org.eclipse.angus.mail.imap.IMAPFolder
import org.eclipse.angus.mail.imap.IMAPStore
import org.eclipse.angus.mail.imap.protocol.BASE64MailboxEncoder
import org.eclipse.angus.mail.imap.protocol.BODY
import org.eclipse.angus.mail.imap.protocol.BODYSTRUCTURE
import org.eclipse.angus.mail.imap.protocol.ENVELOPE
import org.eclipse.angus.mail.imap.protocol.FLAGS
import org.eclipse.angus.mail.imap.protocol.FetchResponse
import org.eclipse.angus.mail.imap.protocol.IMAPProtocol
import org.eclipse.angus.mail.imap.protocol.IMAPResponse
import org.eclipse.angus.mail.imap.protocol.INTERNALDATE
import org.eclipse.angus.mail.imap.protocol.RFC822SIZE
import org.eclipse.angus.mail.imap.protocol.UID
import org.eclipse.angus.mail.smtp.SMTPMessage
import org.eclipse.angus.mail.smtp.SMTPTransport

/**
 * A real account over IMAP and SMTP with Angus Mail, ported from desktop's ImapAccount (imap.js,
 * spec 4 §2-3). Two connections ("lines"): main does syncs, flags, moves and appends; reader only
 * fetches the message the user opens, so opening one never waits behind a sync. Commands on a line
 * run one at a time, user work before background work. Each command opens the folder it needs and
 * closes it again; sync, previews and moves are raw IMAP commands, so they match desktop's exactly.
 */
class ImapSession(
    private val account: Account,
    private val credentials: Credentials,
    private val strings: MailStrings,
    private val connectTimeoutMs: Int = 20_000,
    private val socketTimeoutMs: Int = 5 * 60_000,
    /** Where IMAP commands run (ImapThreads); tests pass threads with a small stack to overflow it quickly. */
    private val commandThreads: CoroutineDispatcher = ImapThreads.dispatcher,
) : MailSession {
    private class Line {
        val lock = PriorityLock()

        @Volatile
        var store: IMAPStore? = null
    }

    private val main = Line()
    private val reader = Line()

    /** Set when the server turned the reader down: reads then go over the main connection. */
    @Volatile
    private var oneConnection = false

    /** Set by close(): nothing may connect through this session again. */
    @Volatile
    private var closed = false

    private val imap: ServerConfig get() = account.imap ?: throw MailException(strings.get(R.string.engine_errors_account_missing))
    private val smtp: ServerConfig get() = account.smtp ?: throw MailException(strings.get(R.string.engine_errors_account_missing))

    private fun closedError() = MailException(strings.get(R.string.engine_errors_connection_closed), "closed")

    private suspend fun secret(): String = when (val c = credentials) {
        is Credentials.Password -> c.password
        is Credentials.OAuth -> c.accessToken()
    }

    private fun tls(props: Properties, protocol: String, server: ServerConfig) {
        if (server.allowInvalidCert) {
            props.setProperty("mail.$protocol.ssl.trust", "*")
            props.setProperty("mail.$protocol.ssl.checkserveridentity", "false")
        } else {
            props.setProperty("mail.$protocol.ssl.checkserveridentity", "true")
            // OkHttp's verifier behaves the same on Android and the JVM; Angus's default leans on JDK internals.
            props[("mail.$protocol.ssl.hostnameverifier")] = OkHostnameVerifier
        }
    }

    private fun imapProps(): Properties {
        val server = imap
        val protocol = if (server.secure) "imaps" else "imap"
        return Properties().apply {
            setProperty("mail.store.protocol", protocol)
            setProperty("mail.$protocol.host", server.host)
            setProperty("mail.$protocol.port", server.port.toString())
            setProperty("mail.$protocol.connectiontimeout", connectTimeoutMs.toString())
            setProperty("mail.$protocol.timeout", socketTimeoutMs.toString())
            setProperty("mail.$protocol.peek", "true")
            // Like imapflow: upgrade a plain connection when the server offers STARTTLS.
            if (!server.secure) setProperty("mail.imap.starttls.enable", "true")
            if (credentials is Credentials.OAuth) setProperty("mail.$protocol.auth.mechanisms", "XOAUTH2")
            tls(this, protocol, server)
        }
    }

    private fun smtpProps(): Properties {
        val server = smtp
        val protocol = if (server.secure) "smtps" else "smtp"
        return Properties().apply {
            setProperty("mail.transport.protocol", protocol)
            setProperty("mail.$protocol.host", server.host)
            setProperty("mail.$protocol.port", server.port.toString())
            setProperty("mail.$protocol.auth", "true")
            setProperty("mail.$protocol.connectiontimeout", connectTimeoutMs.toString())
            setProperty("mail.$protocol.timeout", "60000")
            // The phone has no host name a server would accept in EHLO; nodemailer falls back to the same literal.
            setProperty("mail.$protocol.localhost", "[127.0.0.1]")
            // STARTTLS is mandatory when the port isn't implicit TLS (nodemailer requireTLS).
            if (!server.secure) {
                setProperty("mail.smtp.starttls.enable", "true")
                setProperty("mail.smtp.starttls.required", "true")
            }
            if (credentials is Credentials.OAuth) setProperty("mail.$protocol.auth.mechanisms", "XOAUTH2")
            tls(this, protocol, server)
        }
    }

    private suspend fun openStore(): IMAPStore {
        val server = imap
        val user = server.user.ifBlank { account.email }
        val password = secret()
        val props = imapProps()
        val store = Session.getInstance(props).getStore(props.getProperty("mail.store.protocol")) as IMAPStore
        try {
            withContext(Dispatchers.IO) { store.connect(server.host, server.port, user, password) }
        } catch (e: Exception) {
            // A rejected login can leave the TLS socket open.
            withContext(Dispatchers.IO) { runCatching { store.close() } }
            throw e
        }
        return store
    }

    private suspend fun connect(line: Line): IMAPStore {
        line.store?.let { if (withContext(Dispatchers.IO) { it.isConnected }) return it }
        line.store = null
        val store = openStore()
        // Closed while it connected: the session is gone, so the connection goes too.
        if (closed) {
            withContext(Dispatchers.IO) { runCatching { store.close() } }
            throw closedError()
        }
        line.store = store
        return store
    }

    private suspend fun <T> execute(line: Line, store: IMAPStore, block: (IMAPStore) -> T): T {
        if (closed) throw closedError()
        return try {
            withContext(commandThreads) { block(store) }
        } catch (e: Exception) {
            if (!withContext(Dispatchers.IO) { runCatching { store.isConnected }.getOrDefault(false) }) line.store = null
            throw e
        } catch (e: StackOverflowError) {
            // The response reader stopped halfway through a reply, so the connection can't be trusted for the next command.
            withContext(Dispatchers.IO) { runCatching { store.close() } }
            line.store = null
            throw MailException(strings.get(R.string.engine_errors_connection_too_deep), TOO_DEEP)
        }
    }

    /** Work on the main connection; [background] work waits for everything the user is waiting for. */
    private suspend fun <T> run(background: Boolean = false, block: (IMAPStore) -> T): T = main.lock.withLock(background) {
        if (closed) throw closedError()
        val store = try {
            connect(main)
        } catch (e: Exception) {
            // A server that takes one connection at a time turns down whichever connects second: the reader gives
            // its connection up and everything goes over this one from now on.
            val readerStore = reader.store ?: throw e
            oneConnection = true
            reader.store = null
            withContext(Dispatchers.IO) { runCatching { readerStore.close() } }
            connect(main)
        }
        execute(main, store, block)
    }

    private sealed interface Attempt<out T> {
        class Done<T>(val value: T) : Attempt<T>

        data object Fallback : Attempt<Nothing>
    }

    /** Fetching the message the user opens, over the reader connection that nothing else uses. */
    private suspend fun <T> read(block: (IMAPStore) -> T): T {
        if (!oneConnection) {
            val attempt: Attempt<T> = reader.lock.withLock {
                if (closed) throw closedError()
                val store = try {
                    connect(reader)
                } catch (e: Exception) {
                    if (main.store == null) throw e
                    oneConnection = true
                    return@withLock Attempt.Fallback
                }
                Attempt.Done(execute(reader, store, block))
            }
            if (attempt is Attempt.Done) return attempt.value
        }
        return run(false, block)
    }

    override suspend fun verify() {
        try {
            val store = openStore()
            withContext(Dispatchers.IO) { runCatching { store.close() } }
        } catch (e: Exception) {
            throw MailException("IMAP: ${FriendlyErrors.message(e, strings)}", "imap", e)
        }
        try {
            val transport = connectTransport()
            withContext(Dispatchers.IO) { runCatching { transport.second.close() } }
        } catch (e: Exception) {
            throw MailException("SMTP: ${FriendlyErrors.message(e, strings)}", "smtp", e)
        }
    }

    override suspend fun listFolders(background: Boolean): List<FolderRecord> = run(background) { store ->
        store.defaultFolder.list("*").mapNotNull { f ->
            val folder = f as IMAPFolder
            val attributes = runCatching { folder.attributes?.toList() }.getOrNull().orEmpty()
            if (attributes.any { it.equals("\\Noselect", true) || it.equals("\\NonExistent", true) }) return@mapNotNull null
            val special = attributes.firstOrNull { it.lowercase() in SPECIAL_USE }
            FolderRecord(
                path = folder.fullName,
                name = folder.name,
                delimiter = runCatching { folder.separator.toString() }.getOrNull(),
                specialUse = special,
                role = FolderRoles.roleFor(folder.fullName, folder.name, special, attributes),
            )
        }
    }

    override suspend fun createFolder(name: String): String = run { store ->
        val namespace = runCatching { store.personalNamespaces.firstOrNull() }.getOrNull()
        val prefix = namespace?.fullName?.takeIf { it.isNotEmpty() }?.let { it + namespace.separator } ?: ""
        val folder = store.getFolder(prefix + name)
        if (!folder.create(Folder.HOLDS_MESSAGES)) throw MailException(strings.get(R.string.engine_errors_folder_duplicate), "exists")
        folder.fullName
    }

    /** Runs raw commands on an open folder; NO and BAD answers throw. */
    private fun <T> IMAPFolder.protocol(block: (IMAPProtocol) -> T): T {
        @Suppress("UNCHECKED_CAST")
        return doCommand { p -> block(p) } as T
    }

    private fun IMAPProtocol.exec(command: String, args: Argument? = null): Array<Response> {
        val responses = command(command, args)
        notifyResponseHandlers(responses)
        handleResult(responses.last())
        return responses
    }

    private fun IMAPProtocol.mailbox(name: String): Argument =
        Argument().apply { if (supportsUtf8()) writeString(name, Charsets.UTF_8) else writeString(BASE64MailboxEncoder.encode(name)) }

    private fun <T> withFolder(store: IMAPStore, path: String, mode: Int, block: (IMAPFolder) -> T): T {
        val folder = store.getFolder(path) as IMAPFolder
        folder.open(mode)
        try {
            return block(folder)
        } finally {
            runCatching { if (folder.isOpen) folder.close(false) }
        }
    }

    private class Detail(val envelope: ENVELOPE?, val structure: BODYSTRUCTURE?, val internalDate: Long?, val size: Long)

    override suspend fun syncFolder(
        path: String,
        limit: Int,
        known: Map<Long, CachedMessage>,
        uidValidity: String?,
        background: Boolean,
        signals: Boolean,
        gmailTabs: Boolean,
        olderUnseen: Boolean,
    ): FolderSync {
        // New messages whose structure overflows the parser: they are fetched without it (see tooDeep).
        var bare = emptySet<Long>()
        while (true) {
            var fresh = emptyList<Long>()
            try {
                return syncOnce(path, limit, known, uidValidity, background, signals, gmailTabs, olderUnseen, bare) { fresh = it }
            } catch (e: MailException) {
                if (e.code != TOO_DEEP) throw e
                // One overflow loses the whole batch, and the next sync would ask for the same batch again.
                val deep = tooDeep(path, fresh - bare, background)
                if (deep.isEmpty()) throw e
                bare = bare + deep
            }
        }
    }

    /**
     * Of [uids], which overflowed the parser when fetched together, the ones whose structure overflows it alone. Each
     * half is fetched with nothing but its structure, and a half that overflows is halved again. Every overflow costs
     * the connection (execute), so this only runs after a sync failed on one.
     */
    private suspend fun tooDeep(path: String, uids: List<Long>, background: Boolean): Set<Long> {
        if (uids.size <= 1) return uids.toSet()
        val out = HashSet<Long>()
        for (half in uids.chunked((uids.size + 1) / 2)) {
            try {
                run(background) { store ->
                    withFolder(store, path, Folder.READ_ONLY) { folder -> folder.protocol { p -> p.exec("UID FETCH ${uidSet(half)} (UID BODYSTRUCTURE)") } }
                }
            } catch (e: MailException) {
                if (e.code != TOO_DEEP) throw e
                out += tooDeep(path, half, background)
            }
        }
        return out
    }

    /** One sync of [path]; the uids in [bare] are fetched without their structure. [onFresh] gets the uids the cache lacks. */
    private suspend fun syncOnce(
        path: String,
        limit: Int,
        known: Map<Long, CachedMessage>,
        uidValidity: String?,
        background: Boolean,
        signals: Boolean,
        gmailTabs: Boolean,
        olderUnseen: Boolean,
        bare: Set<Long>,
        onFresh: (List<Long>) -> Unit,
    ): FolderSync =
        run(background) { store ->
            withFolder(store, path, Folder.READ_ONLY) { folder ->
                val validity = folder.uidValidity.toString()
                // After a UIDVALIDITY change old UIDs point at different messages; reuse nothing.
                val cache = if (uidValidity != null && uidValidity == validity) known else emptyMap()
                val exists = folder.messageCount
                if (exists <= 0) return@withFolder FolderSync(validity, 0, emptyList(), if (olderUnseen) 0 else null)
                val start = maxOf(1, exists - limit + 1)
                folder.protocol { p ->
                    // The flags of every message in the window: all that can change about a message the cache has.
                    val rows = p.exec("FETCH $start:* (UID FLAGS)").filterIsInstance<FetchResponse>()
                        .mapNotNull { fr -> fr.getItem(UID::class.java)?.let { Triple(fr.number, it.uid, fr.getItem(FLAGS::class.java) ?: Flags()) } }
                        .sortedBy { it.first }
                        .distinctBy { it.second }
                    // Envelope, structure, date and size only for messages the cache doesn't have yet: on a large Gmail
                    // folder that is the slow part.
                    val fresh = rows.map { it.second }.filter { it !in cache }
                    onFresh(fresh)
                    val details = mutableMapOf<Long, Detail>()
                    // Without a structure a message is listed with its sender and subject, but no preview or attachment mark.
                    val (flat, full) = fresh.partition { it in bare }
                    for ((uids, items) in listOf(full to "ENVELOPE BODYSTRUCTURE", flat to "ENVELOPE")) {
                        if (uids.isEmpty()) continue
                        for (fr in p.exec("UID FETCH ${uidSet(uids)} (UID $items INTERNALDATE RFC822.SIZE)").filterIsInstance<FetchResponse>()) {
                            val uid = fr.getItem(UID::class.java)?.uid ?: continue
                            details[uid] = Detail(
                                fr.getItem(ENVELOPE::class.java),
                                fr.getItem(BODYSTRUCTURE::class.java),
                                fr.getItem(INTERNALDATE::class.java)?.date?.time,
                                fr.getItem(RFC822SIZE::class.java)?.size ?: 0,
                            )
                        }
                    }
                    val previews = previews(p, details)
                    // A few header lines, for new messages and for ones cached before Rukoo kept them.
                    val kinds = if (signals) signalsOf(p, rows.map { it.second }.filter { it in details || cache[it]?.signals == null }) else emptyMap()
                    // Gmail doesn't show its tabs as folders, but its own search finds them (X-GM-RAW). Asked again on every
                    // sync, since a message can move to another tab. What none of them holds is in Primary.
                    val tabs = if (gmailTabs && rows.isNotEmpty() && p.hasCapability("X-GM-EXT-1")) gmailTabsOf(p, rows.minOf { it.second }) else null
                    // A window that starts at the first message leaves nothing older.
                    val older = if (!olderUnseen) null else if (start == 1) 0 else unseenBelow(p, rows.minOfOrNull { it.second })
                    fun CachedMessage.extra(prior: CachedMessage?): CachedMessage = copy(
                        signals = kinds[uid] ?: prior?.signals,
                        gmailTab = if (tabs != null) tabs[uid] ?: "primary" else prior?.gmailTab,
                    )
                    val messages = rows.mapNotNull { (_, uid, flags) ->
                        val unread = !flags.contains(Flags.Flag.SEEN)
                        val starred = flags.contains(Flags.Flag.FLAGGED)
                        val answered = flags.contains(Flags.Flag.ANSWERED)
                        val detail = details[uid]
                        val prior = cache[uid]
                        when {
                            detail != null -> fromDetail(uid, detail, previews[uid] ?: prior?.preview.orEmpty(), unread, starred, answered).extra(prior)
                            prior != null -> prior.copy(unread = unread, starred = starred, answered = answered).extra(prior)
                            // Gone between the two fetches (moved or deleted meanwhile): the next sync no longer lists it.
                            else -> null
                        }
                    }
                    FolderSync(validity, exists, messages, older)
                }
            }
        }

    /** The signal headers of [uids] (MailSignals.HEADERS), read into signals. */
    private fun signalsOf(p: IMAPProtocol, uids: List<Long>): Map<Long, MailSignals> {
        if (uids.isEmpty()) return emptyMap()
        val fields = MailSignals.HEADERS.joinToString(" ") { it.uppercase() }
        val out = HashMap<Long, MailSignals>()
        // The first 8 KiB of them: anyone can send mail with huge headers, and a batch holds up to 300 messages.
        for (fr in p.exec("UID FETCH ${uidSet(uids)} (UID BODY.PEEK[HEADER.FIELDS ($fields)]<0.$SIGNAL_BYTES>)").filterIsInstance<FetchResponse>()) {
            val uid = fr.getItem(UID::class.java)?.uid ?: continue
            val bytes = fr.getItem(BODY::class.java)?.byteArray?.newBytes ?: ByteArray(0)
            out[uid] = MailSignals.parse(String(bytes, Charsets.UTF_8))
        }
        return out
    }

    /** The Gmail tab of each message from uid [first] on, or null when Gmail's search failed: then the cached tabs stay. */
    private fun gmailTabsOf(p: IMAPProtocol, first: Long): Map<Long, String>? = try {
        val tabs = HashMap<Long, String>()
        for (tab in GMAIL_TABS) {
            for (uid in searchUids(p.exec("UID SEARCH UID $first:* X-GM-RAW \"category:$tab\""))) tabs[uid] = tab
        }
        tabs
    } catch (e: Exception) {
        null
    }

    /**
     * How many messages of the open folder are unread below uid [first], the oldest of the window a sync fetches; null
     * when the server doesn't say (imap.js unseenBelow). Only below the window: the window's own unread mail is counted
     * in the cache, where reading a message counts at once. With ESEARCH the server sends only the count; otherwise the
     * uids it sends are counted.
     */
    private fun unseenBelow(p: IMAPProtocol, first: Long?): Int? {
        if (first == null) return null
        if (first <= 1) return 0
        val below = "UID 1:${first - 1} UNSEEN"
        return try {
            searchCount(p.exec(if (p.hasCapability("ESEARCH")) "UID SEARCH RETURN (COUNT) $below" else "UID SEARCH $below"))
        } catch (e: Exception) {
            null
        }
    }

    /** A few KB of the readable part of each new message, one fetch per part number (imap.js:281-306). */
    private fun previews(p: IMAPProtocol, details: Map<Long, Detail>): Map<Long, String> {
        val byPart = mutableMapOf<String, MutableList<Pair<Long, PartNode>>>()
        for ((uid, d) in details) {
            val node = Structure.findPreviewPart(d.structure?.let { partNode(it, null) }) ?: continue
            byPart.getOrPut(Structure.partKey(node)) { mutableListOf() } += uid to node
        }
        val out = mutableMapOf<Long, String>()
        for ((key, items) in byPart) {
            val nodes = items.toMap()
            val responses = runCatching { p.exec("UID FETCH ${uidSet(items.map { it.first })} (UID BODY.PEEK[$key]<0.$PREVIEW_BYTES>)") }.getOrNull() ?: continue
            for (fr in responses.filterIsInstance<FetchResponse>()) {
                val uid = fr.getItem(UID::class.java)?.uid ?: continue
                val node = nodes[uid] ?: continue
                val bytes = fr.getItem(BODY::class.java)?.byteArray?.newBytes ?: continue
                out[uid] = runCatching { MailText.previewFromPart(bytes, node.type, node.encoding, node.charset) }.getOrDefault("")
            }
        }
        return out
    }

    private fun addresses(list: Array<InternetAddress>?): List<Address> = list.orEmpty().flatMap { a ->
        if (runCatching { a.isGroup }.getOrDefault(false)) runCatching { a.getGroup(false)?.toList() }.getOrNull().orEmpty() else listOf(a)
    }.map { Address(it.personal.orEmpty(), it.address.orEmpty()) }

    private fun fromDetail(uid: Long, d: Detail, preview: String, unread: Boolean, starred: Boolean, answered: Boolean): CachedMessage {
        val env = d.envelope
        val subject = env?.subject?.let { s -> runCatching { MimeUtility.decodeText(MimeUtility.unfold(s)) }.getOrDefault(s) }.orEmpty()
        return CachedMessage(
            uid = uid,
            messageId = env?.messageId?.trim()?.takeIf { it.isNotEmpty() },
            inReplyTo = env?.inReplyTo?.trim()?.takeIf { it.isNotEmpty() },
            subject = subject,
            from = addresses(env?.from).firstOrNull() ?: Address(),
            to = addresses(env?.to),
            cc = addresses(env?.cc),
            replyTo = addresses(env?.replyTo),
            date = env?.date?.time ?: d.internalDate ?: System.currentTimeMillis(),
            preview = preview,
            unread = unread,
            starred = starred,
            answered = answered,
            hasAttachments = Structure.hasAttachments(d.structure?.let { partNode(it, null) }),
            size = d.size,
        )
    }

    override suspend fun fetchSource(path: String, uid: Long, background: Boolean): ByteArray {
        val fetch = { store: IMAPStore ->
            withFolder(store, path, Folder.READ_ONLY) { folder ->
                folder.protocol { p ->
                    // PEEK never sets \Seen; marking read is the reader's explicit choice.
                    p.exec("UID FETCH $uid (UID BODY.PEEK[])").filterIsInstance<FetchResponse>()
                        .firstNotNullOfOrNull { fr -> fr.getItem(BODY::class.java)?.byteArray?.newBytes }
                }
            } ?: throw MailException(strings.get(R.string.engine_errors_message_server_missing), "missing")
        }
        return if (background) run(true, fetch) else read(fetch)
    }

    override suspend fun setFlags(path: String, uids: List<Long>, unread: Boolean?, starred: Boolean?, answered: Boolean?) {
        if (uids.isEmpty()) return
        run { store ->
            withFolder(store, path, Folder.READ_WRITE) { folder ->
                val set = uidSet(uids)
                folder.protocol { p ->
                    if (unread == true) p.exec("UID STORE $set -FLAGS.SILENT (\\Seen)")
                    if (unread == false) p.exec("UID STORE $set +FLAGS.SILENT (\\Seen)")
                    if (starred == true) p.exec("UID STORE $set +FLAGS.SILENT (\\Flagged)")
                    if (starred == false) p.exec("UID STORE $set -FLAGS.SILENT (\\Flagged)")
                    // \Answered is only ever added, like desktop.
                    if (answered == true) p.exec("UID STORE $set +FLAGS.SILENT (\\Answered)")
                }
            }
        }
    }

    override suspend fun move(path: String, uids: List<Long>, destination: String, expectUidValidity: String?): MoveOutcome = run { store ->
        withFolder(store, path, Folder.READ_WRITE) { folder ->
            // Refuse a mailbox whose uids were reassigned since: the uid would point at another message.
            if (expectUidValidity != null && folder.uidValidity.toString() != expectUidValidity) throw UidValidityChanged()
            val set = uidSet(uids)
            folder.protocol { p ->
                val responses = if (p.hasCapability("MOVE")) {
                    p.exec("UID MOVE $set", p.mailbox(destination)).toList()
                } else {
                    // Without MOVE: copy, flag deleted and expunge, like imapflow.
                    val copied = p.exec("UID COPY $set", p.mailbox(destination)).toList()
                    p.exec("UID STORE $set +FLAGS.SILENT (\\Deleted)")
                    if (p.hasCapability("UIDPLUS")) p.exec("UID EXPUNGE $set") else p.exec("EXPUNGE")
                    copied
                }
                val copy = responses.firstNotNullOfOrNull { CopyUid.parse(it.toString()) }
                MoveOutcome(copy?.map.orEmpty(), copy?.uidValidity)
            }
        }
    }

    override suspend fun deleteForever(path: String, uids: List<Long>) {
        if (uids.isEmpty()) return
        run { store ->
            withFolder(store, path, Folder.READ_WRITE) { folder ->
                val set = uidSet(uids)
                folder.protocol { p ->
                    p.exec("UID STORE $set +FLAGS.SILENT (\\Deleted)")
                    if (p.hasCapability("UIDPLUS")) p.exec("UID EXPUNGE $set") else p.exec("EXPUNGE")
                }
            }
        }
    }

    override suspend fun append(path: String, raw: ByteArray, flags: List<String>): Long? = run { store ->
        val folder = store.getFolder(path) as IMAPFolder
        val message = MimeMessage(Session.getInstance(Properties()), ByteArrayInputStream(raw))
        val set = Flags()
        for (f in flags) {
            when (f.lowercase()) {
                "\\seen" -> set.add(Flags.Flag.SEEN)
                "\\draft" -> set.add(Flags.Flag.DRAFT)
                "\\flagged" -> set.add(Flags.Flag.FLAGGED)
                "\\answered" -> set.add(Flags.Flag.ANSWERED)
            }
        }
        message.setFlags(set, true)
        folder.appendUIDMessages(arrayOf(message))?.firstOrNull()?.uid?.takeIf { it > 0 }
    }

    private suspend fun connectTransport(): Pair<Session, SMTPTransport> {
        val server = smtp
        val user = server.user.ifBlank { account.email }
        val password = secret()
        val props = smtpProps()
        val session = Session.getInstance(props)
        val transport = session.getTransport(props.getProperty("mail.transport.protocol")) as SMTPTransport
        try {
            withContext(Dispatchers.IO) { transport.connect(server.host, server.port, user, password) }
        } catch (e: Exception) {
            withContext(Dispatchers.IO) { runCatching { transport.close() } }
            throw e
        }
        return session to transport
    }

    override suspend fun send(mail: ComposedMail): ByteArray {
        val raw = MimeComposer.compose(mail)
        val (session, transport) = try {
            connectTransport()
        } catch (e: Exception) {
            throw FriendlyErrors.wrap(e, strings)
        }
        try {
            withContext(Dispatchers.IO) {
                val message = SMTPMessage(session, ByteArrayInputStream(raw)).apply { envelopeFrom = mail.from.address }
                val recipients = mail.recipients.map { InternetAddress().apply { address = it.address.trim() } }.toTypedArray()
                transport.sendMessage(message, recipients)
            }
        } catch (e: Exception) {
            throw FriendlyErrors.wrap(e, strings)
        } finally {
            withContext(Dispatchers.IO) { runCatching { transport.close() } }
        }
        return raw
    }

    override fun savesSentCopy(): Boolean = AUTO_SENT_HOSTS.containsMatchIn(account.smtp?.host.orEmpty())

    override suspend fun close() {
        closed = true
        val stores = listOfNotNull(main.store, reader.store)
        main.store = null
        reader.store = null
        withContext(Dispatchers.IO) { stores.forEach { runCatching { it.close() } } }
    }

    companion object {
        const val PREVIEW_BYTES = 4096
        const val SIGNAL_BYTES = 8192

        /** The code of the error a reply nested too deeply to parse becomes. */
        private const val TOO_DEEP = "too-deep"

        /** Hosts whose SMTP service already files a copy in the Sent folder (imap.js:20). */
        val AUTO_SENT_HOSTS = Regex("(gmail\\.com|googlemail\\.com|office365\\.com|outlook\\.com)$", RegexOption.IGNORE_CASE)

        /** The tabs Gmail sorts its inbox into besides Primary, as its search names them. */
        val GMAIL_TABS = listOf("promotions", "social", "updates", "forums")

        /** The uids of the SEARCH lines in a command's responses. */
        fun searchUids(responses: Array<Response>): List<Long> {
            val out = ArrayList<Long>()
            for (r in responses) {
                if (r !is IMAPResponse || !r.keyEquals("SEARCH")) continue
                while (true) {
                    val n = r.readLong()
                    if (n < 0) break
                    out += n
                }
            }
            return out
        }

        private val ESEARCH_COUNT = Regex("\\bCOUNT (\\d+)", RegexOption.IGNORE_CASE)

        /**
         * How many messages a search found: the COUNT of its ESEARCH answer (RFC 4731), 0 when that answer names none, or
         * else the uids of its SEARCH lines.
         */
        fun searchCount(responses: Array<Response>): Int {
            for (r in responses) {
                if (r is IMAPResponse && r.keyEquals("ESEARCH")) return ESEARCH_COUNT.find(r.toString())?.groupValues?.get(1)?.toIntOrNull() ?: 0
            }
            return searchUids(responses).size
        }

        private val SPECIAL_USE = setOf("\\inbox", "\\sent", "\\drafts", "\\trash", "\\junk", "\\archive", "\\all", "\\flagged", "\\important")

        /** A UID set with ranges: 1,2,3,5 → 1:3,5. */
        fun uidSet(uids: Collection<Long>): String {
            val sorted = uids.toSortedSet().toList()
            val out = StringBuilder()
            var i = 0
            while (i < sorted.size) {
                var j = i
                while (j + 1 < sorted.size && sorted[j + 1] == sorted[j] + 1) j++
                if (out.isNotEmpty()) out.append(',')
                out.append(sorted[i])
                if (j > i) out.append(':').append(sorted[j])
                i = j + 1
            }
            return out.toString()
        }

        /**
         * BODYSTRUCTURE as [PartNode]s with IMAP part numbers ("1", "1.2"); a single-part body has none. Below
         * [Structure.MAX_DEPTH] multipart levels the parts are left out, as the parser leaves them out of the body.
         */
        fun partNode(bs: BODYSTRUCTURE, part: String?, depth: Int = 0): PartNode {
            if (bs.isMulti) {
                return PartNode(
                    part = part,
                    type = "multipart/${bs.subtype.orEmpty().lowercase()}",
                    children = if (depth >= Structure.MAX_DEPTH) emptyList() else bs.bodies.orEmpty().mapIndexed { i, child ->
                        partNode(child, if (part == null) "${i + 1}" else "$part.${i + 1}", depth + 1)
                    },
                )
            }
            return PartNode(
                part = part,
                type = "${bs.type.orEmpty()}/${bs.subtype.orEmpty()}".lowercase(),
                disposition = bs.disposition,
                id = bs.id,
                encoding = bs.encoding,
                charset = bs.cParams?.get("charset"),
                filename = bs.dParams?.get("filename"),
                name = bs.cParams?.get("name"),
            )
        }
    }
}

/** The COPYUID response code (RFC 4315): the destination's UIDVALIDITY and the old → new uid map. */
class CopyUid(val uidValidity: String, val map: Map<Long, Long>) {
    companion object {
        private val CODE = Regex("\\[COPYUID (\\d+) ([0-9:,]+) ([0-9:,]+)]", RegexOption.IGNORE_CASE)

        fun expand(set: String): List<Long> = set.split(',').flatMap { range ->
            val a = range.substringBefore(':').toLongOrNull() ?: return@flatMap emptyList()
            val b = range.substringAfter(':', range).toLongOrNull() ?: a
            if (b >= a) (a..b).toList() else (a downTo b).toList()
        }

        fun parse(line: String): CopyUid? {
            val m = CODE.find(line) ?: return null
            val from = expand(m.groupValues[2])
            val to = expand(m.groupValues[3])
            if (from.size != to.size) return null
            return CopyUid(m.groupValues[1], from.zip(to).toMap())
        }
    }
}

/**
 * Threads for IMAP commands. Angus Mail parses a message's BODYSTRUCTURE recursively, without a depth limit, so a
 * crafted, deeply nested message could overflow a normal thread's stack during sync. These threads have a large
 * stack; an overflow that happens anyway becomes a mail error in [ImapSession], and a sync lists that message
 * without its structure.
 */
internal object ImapThreads {
    private const val STACK_BYTES = 32L * 1024 * 1024

    val dispatcher: CoroutineDispatcher = withStack(STACK_BYTES)

    /** Daemon threads with [stackBytes] of stack each. */
    fun withStack(stackBytes: Long): ExecutorCoroutineDispatcher = Executors.newCachedThreadPool { work ->
        Thread(null, work, "rukoo-imap", stackBytes).apply { isDaemon = true }
    }.asCoroutineDispatcher()
}
