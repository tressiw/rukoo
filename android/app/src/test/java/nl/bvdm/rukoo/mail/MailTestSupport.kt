package nl.bvdm.rukoo.mail

import java.io.File
import javax.xml.parsers.DocumentBuilderFactory
import nl.bvdm.rukoo.R
import nl.bvdm.rukoo.core.SecretCipher
import nl.bvdm.rukoo.mail.mime.ComposedMail
import nl.bvdm.rukoo.mail.mime.MimeComposer
import nl.bvdm.rukoo.mail.model.Address
import nl.bvdm.rukoo.mail.model.CachedMessage
import nl.bvdm.rukoo.mail.model.FolderRole
import nl.bvdm.rukoo.mail.session.FolderRecord
import nl.bvdm.rukoo.mail.session.FolderSync
import nl.bvdm.rukoo.mail.session.MailSession
import nl.bvdm.rukoo.mail.session.MoveOutcome
import nl.bvdm.rukoo.mail.session.UidValidityChanged
import org.w3c.dom.Element

/** The English texts of strings_mail_engine.xml, found by resource id through the generated R class. */
object TestStrings : MailStrings {
    private val file = listOf("src/main/res/values/strings_mail_engine.xml", "app/src/main/res/values/strings_mail_engine.xml")
        .map(::File).first { it.exists() }
    private val doc = DocumentBuilderFactory.newInstance().newDocumentBuilder().parse(file)

    private fun unescape(s: String) = s.replace("\\'", "'").replace("\\\"", "\"").replace("\\\\", "\\")

    private val texts: Map<String, String> = doc.getElementsByTagName("string").let { list ->
        (0 until list.length).associate { i -> (list.item(i) as Element).let { it.getAttribute("name") to unescape(it.textContent) } }
    }
    private val plurals: Map<String, Map<String, String>> = doc.getElementsByTagName("plurals").let { list ->
        (0 until list.length).associate { i ->
            val el = list.item(i) as Element
            val items = el.getElementsByTagName("item")
            el.getAttribute("name") to (0 until items.length).associate { j -> (items.item(j) as Element).let { it.getAttribute("quantity") to unescape(it.textContent) } }
        }
    }
    private val stringNames: Map<Int, String> = R.string::class.java.fields.associate { it.getInt(null) to it.name }
    private val pluralNames: Map<Int, String> = R.plurals::class.java.fields.associate { it.getInt(null) to it.name }

    override fun get(id: Int, vararg args: Any): String {
        val text = texts[stringNames[id]] ?: error("no English text for ${stringNames[id] ?: id}")
        return if (args.isEmpty()) text else String.format(text, *args)
    }

    override fun plural(id: Int, count: Int, vararg args: Any): String {
        val forms = plurals[pluralNames[id]] ?: error("no plural ${pluralNames[id] ?: id}")
        return String.format(forms[if (count == 1) "one" else "other"] ?: forms.getValue("other"), *args)
    }
}

/** Reversible and visible, so tests can see that secrets are stored encrypted. */
object FakeCipher : SecretCipher {
    override fun encrypt(plain: String) = "enc:" + plain.reversed()

    override fun decrypt(blob: String) = if (blob.startsWith("enc:")) blob.removePrefix("enc:").reversed() else null
}

/** A message on the fake server. */
data class ServerMessage(
    val uid: Long,
    val subject: String,
    val from: Address = Address("Sender", "sender@example.com"),
    val to: List<Address> = listOf(Address("Me", "me@example.com")),
    val date: Long = 1_700_000_000_000,
    val messageId: String = "<m$uid@example.com>",
    val text: String = "Body of $subject",
    val flags: Set<String> = emptySet(),
    /** The stored source, for messages that were appended. */
    val raw: ByteArray? = null,
)

class ServerBox(var uidValidity: String = "1", val messages: MutableList<ServerMessage> = mutableListOf())

/**
 * An IMAP server in memory: folders, uids, flags, moves with or without UIDPLUS, and failures on
 * demand. Records which uids each sync fetched in full, to check the cache window logic.
 */
class FakeServer(val folders: List<FolderRecord>) {
    val boxes = folders.associate { it.path to ServerBox() }.toMutableMap()
    var nextUid = 1000L
    var uidplus = true
    /** Makes the next call of one operation ("verify", "setFlags", "move", ...) throw. */
    var failNext: Pair<String, Exception>? = null
    val fullyFetched = mutableListOf<Long>()

    /** How long a move takes, to cancel a caller while it runs. */
    var moveDelayMs = 0L

    /** How long a flag change takes before it succeeds or fails, to overlap two of them. */
    var setFlagsDelayMs = 0L

    /** Folders whose every sync throws, to keep one failing while the others work. */
    val failingFolders = mutableMapOf<String, Exception>()

    /** How long a folder sync takes, to close the engine while one runs. */
    var syncDelayMs = 0L

    /** The folders whose syncs asked how much mail older than the window is unread. */
    val olderAsked = mutableListOf<String>()

    /** Answer that question with null, as when a server turns the search down. */
    var olderUnseenRefused = false

    /** The sessions made so far; like a real one, a closed session syncs no more. */
    var sessionsMade = 0
    val sent = mutableListOf<ComposedMail>()
    val appended = mutableListOf<Pair<String, ByteArray>>()

    fun add(folder: String, m: ServerMessage) {
        boxes.getValue(folder).messages += m
    }

    fun box(path: String) = boxes[path] ?: throw IllegalStateException("no folder $path")

    fun session(): MailSession = object : MailSession {
        private var closed = false

        init {
            sessionsMade++
        }

        private fun maybeFail(op: String) {
            val (name, error) = failNext ?: return
            if (name != op) return
            failNext = null
            throw error
        }

        override suspend fun verify() = maybeFail("verify")

        override suspend fun listFolders(background: Boolean) = folders.also { maybeFail("listFolders") }

        override suspend fun createFolder(name: String): String {
            boxes[name] = ServerBox()
            return name
        }

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
            if (syncDelayMs > 0) kotlinx.coroutines.delay(syncDelayMs)
            if (closed) throw IllegalStateException("this session is closed")
            maybeFail("syncFolder")
            failingFolders[path]?.let { throw it }
            val box = box(path)
            val useKnown = if (uidValidity == box.uidValidity) known else emptyMap()
            val window = box.messages.takeLast(limit)
            if (olderUnseen) olderAsked += path
            return FolderSync(
                box.uidValidity,
                box.messages.size,
                window.map { m ->
                    val flags = CachedMessage(m.uid, unread = "\\Seen" !in m.flags, starred = "\\Flagged" in m.flags, answered = "\\Answered" in m.flags)
                    useKnown[m.uid]?.copy(unread = flags.unread, starred = flags.starred, answered = flags.answered) ?: run {
                        fullyFetched += m.uid
                        CachedMessage(
                            uid = m.uid, messageId = m.messageId, subject = m.subject, from = m.from, to = m.to, date = m.date,
                            preview = m.text, unread = flags.unread, starred = flags.starred, answered = flags.answered, size = m.text.length.toLong(),
                        )
                    }
                },
                olderUnseen = if (!olderUnseen || olderUnseenRefused) null else box.messages.dropLast(limit).count { "\\Seen" !in it.flags },
            )
        }

        override suspend fun fetchSource(path: String, uid: Long, background: Boolean): ByteArray {
            maybeFail("fetchSource")
            val m = box(path).messages.first { it.uid == uid }
            m.raw?.let { return it }
            return MimeComposer.compose(ComposedMail(from = m.from, to = m.to, subject = m.subject, text = m.text, date = m.date, messageId = m.messageId))
        }

        override suspend fun setFlags(path: String, uids: List<Long>, unread: Boolean?, starred: Boolean?, answered: Boolean?) {
            if (setFlagsDelayMs > 0) kotlinx.coroutines.delay(setFlagsDelayMs)
            maybeFail("setFlags")
            val box = box(path)
            box.messages.replaceAll { m ->
                if (m.uid !in uids) return@replaceAll m
                val f = m.flags.toMutableSet()
                if (unread == true) f -= "\\Seen"
                if (unread == false) f += "\\Seen"
                if (starred == true) f += "\\Flagged"
                if (starred == false) f -= "\\Flagged"
                if (answered == true) f += "\\Answered"
                m.copy(flags = f)
            }
        }

        override suspend fun move(path: String, uids: List<Long>, destination: String, expectUidValidity: String?): MoveOutcome {
            maybeFail("move")
            if (moveDelayMs > 0) kotlinx.coroutines.delay(moveDelayMs)
            val src = box(path)
            val dest = box(destination)
            if (expectUidValidity != null && src.uidValidity != expectUidValidity) throw UidValidityChanged()
            val map = mutableMapOf<Long, Long>()
            for (uid in uids) {
                val m = src.messages.first { it.uid == uid }
                src.messages.remove(m)
                val moved = m.copy(uid = nextUid++)
                dest.messages += moved
                map[uid] = moved.uid
            }
            return if (uidplus) MoveOutcome(map, dest.uidValidity) else MoveOutcome(emptyMap(), null)
        }

        override suspend fun deleteForever(path: String, uids: List<Long>) {
            maybeFail("deleteForever")
            box(path).messages.removeAll { it.uid in uids }
        }

        override suspend fun append(path: String, raw: ByteArray, flags: List<String>): Long? {
            appended += path to raw
            val parsed = nl.bvdm.rukoo.mail.mime.MimeParser.parse(raw)
            val uid = nextUid++
            box(path).messages += ServerMessage(uid, parsed.subject.orEmpty(), messageId = parsed.messageId ?: "<x$uid>", flags = flags.toSet(), raw = raw)
            return if (uidplus) uid else null
        }

        override suspend fun send(mail: ComposedMail): ByteArray {
            maybeFail("send")
            sent += mail
            return MimeComposer.compose(mail)
        }

        override fun savesSentCopy() = false

        override suspend fun close() {
            closed = true
        }
    }

    companion object {
        val STANDARD = listOf(
            FolderRecord("INBOX", "INBOX", "/", null, FolderRole.INBOX),
            FolderRecord("Sent", "Sent", "/", null, FolderRole.SENT),
            FolderRecord("Drafts", "Drafts", "/", null, FolderRole.DRAFTS),
            FolderRecord("Trash", "Trash", "/", null, FolderRole.TRASH),
            FolderRecord("Archive", "Archive", "/", null, FolderRole.ARCHIVE),
            FolderRecord("Projects", "Projects", "/", null, FolderRole.NONE),
        )

        val GMAIL = listOf(
            FolderRecord("INBOX", "INBOX", "/", null, FolderRole.INBOX),
            FolderRecord("[Gmail]/All Mail", "All Mail", "/", "\\All", FolderRole.ALL),
            FolderRecord("[Gmail]/Sent Mail", "Sent Mail", "/", "\\Sent", FolderRole.SENT),
            FolderRecord("[Gmail]/Trash", "Trash", "/", "\\Trash", FolderRole.TRASH),
            FolderRecord("[Gmail]/Starred", "Starred", "/", "\\Flagged", FolderRole.FLAGGED),
            FolderRecord("Label", "Label", "/", null, FolderRole.NONE),
        )
    }
}
