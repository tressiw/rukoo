package nl.bvdm.rukoo.mail.demo

import java.io.File
import java.time.ZoneId
import java.util.Base64
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import kotlinx.serialization.Serializable
import nl.bvdm.rukoo.R
import nl.bvdm.rukoo.core.Clock
import nl.bvdm.rukoo.categories.MailSignals
import nl.bvdm.rukoo.core.JsonFiles
import nl.bvdm.rukoo.mail.MailException
import nl.bvdm.rukoo.mail.MailStrings
import nl.bvdm.rukoo.mail.mime.ComposedMail
import nl.bvdm.rukoo.mail.mime.MailAttachment
import nl.bvdm.rukoo.mail.mime.MailText
import nl.bvdm.rukoo.mail.mime.MimeComposer
import nl.bvdm.rukoo.mail.mime.MimeParser
import nl.bvdm.rukoo.mail.model.Account
import nl.bvdm.rukoo.mail.model.Address
import nl.bvdm.rukoo.mail.model.CachedMessage
import nl.bvdm.rukoo.mail.session.FolderRecord
import nl.bvdm.rukoo.mail.session.FolderSync
import nl.bvdm.rukoo.mail.session.MailSession
import nl.bvdm.rukoo.mail.session.MoveOutcome
import nl.bvdm.rukoo.mail.session.UidValidityChanged

/** A file of a demo message, base64 like desktop's demo-server.json. */
@Serializable
data class DemoAttachment(
    val filename: String,
    val contentType: String,
    val content: String,
    val encoding: String = "base64",
    val cid: String? = null,
)

/** A message as the demo server stores it (demo.js:315-330). */
@Serializable
data class DemoMessage(
    val uid: Long,
    val from: Address,
    val to: List<Address> = emptyList(),
    val cc: List<Address> = emptyList(),
    val bcc: List<Address> = emptyList(),
    val subject: String = "",
    val date: Long,
    val html: String? = null,
    val text: String? = null,
    val attachments: List<DemoAttachment> = emptyList(),
    val headers: Map<String, String>? = null,
    val messageId: String,
    val inReplyTo: String? = null,
    val references: List<String>? = null,
    val flags: List<String> = emptyList(),
)

@Serializable
data class DemoBox(val uidValidity: String = "1", val uidNext: Long = 1, val messages: List<DemoMessage> = emptyList())

@Serializable
data class DemoState(val boxes: Map<String, DemoBox> = emptyMap(), val nextUid: Long = 1, val extraFolders: List<String> = emptyList())

/**
 * The demo account's simulated server, ported from desktop's DemoAccount (demo.js:336-518, spec 4 §7).
 * The state lives in demo-server.json, created once from the seed relative to the account's creation,
 * and every change rewrites it. Messages go through real MIME on the way out, so the reader,
 * attachments and unsubscribe paths exercise the same parsing as real mail.
 */
class DemoSession(
    private val file: File,
    private val strings: MailStrings,
    private val clock: Clock = Clock.System,
    private val zone: ZoneId = ZoneId.systemDefault(),
) : MailSession {
    private val mutex = Mutex()
    private var state: DemoState? = null

    private suspend fun <T> withState(write: Boolean = false, block: (DemoState) -> Pair<DemoState, T>): T = mutex.withLock {
        withContext(Dispatchers.IO) {
            val current = state ?: load()
            val (next, result) = block(current)
            state = next
            if (write && next !== current) JsonFiles.write(file, DemoState.serializer(), next)
            result
        }
    }

    private suspend fun <T> read(block: (DemoState) -> T): T = withState { it to block(it) }

    private fun load(): DemoState {
        val stored = runCatching { JsonFiles.read(file, DemoState.serializer(), DemoState()) }.getOrNull()
        if (stored != null && stored.boxes.isNotEmpty()) return stored
        return seed(clock.now(), zone).also { JsonFiles.write(file, DemoState.serializer(), it) }
    }

    private fun DemoState.box(path: String): DemoBox =
        boxes[path] ?: throw MailException(strings.get(R.string.engine_errors_folder_missing, path), "folder")

    private fun DemoBox.find(uid: Long): DemoMessage =
        messages.firstOrNull { it.uid == uid } ?: throw MailException(strings.get(R.string.engine_errors_message_missing), "missing")

    override suspend fun verify() = Unit

    override suspend fun listFolders(background: Boolean): List<FolderRecord> = read { s ->
        PreviewData.FOLDERS.map { (path, role) -> FolderRecord(path, path, "/", null, role) } +
            s.extraFolders.map { FolderRecord(it, it, "/", null) }
    }

    override suspend fun createFolder(name: String): String = withState(write = true) { s ->
        if (name in s.boxes) throw MailException(strings.get(R.string.engine_errors_folder_duplicate), "exists")
        s.copy(boxes = s.boxes + (name to DemoBox()), extraFolders = s.extraFolders + name) to name
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
    ): FolderSync =
        read { s ->
            val box = s.box(path)
            // Like a server's count (ImapSession.unseenBelow): the unread messages older than the window.
            val older = if (olderUnseen) box.messages.dropLast(limit).count { "\\Seen" !in it.flags } else null
            FolderSync(box.uidValidity, box.messages.size, box.messages.takeLast(limit).map(::cached), older)
        }

    private fun cached(m: DemoMessage) = CachedMessage(
        uid = m.uid,
        messageId = m.messageId,
        inReplyTo = m.inReplyTo,
        subject = m.subject,
        from = m.from,
        to = m.to,
        cc = m.cc,
        replyTo = emptyList(),
        date = m.date,
        preview = MailText.makePreview(m.text ?: m.html?.let(MailText::htmlToPlain).orEmpty()),
        unread = "\\Seen" !in m.flags,
        starred = "\\Flagged" in m.flags,
        answered = "\\Answered" in m.flags,
        // Inline pictures referred to by Content-ID are part of the body, as in isAttachmentNode.
        hasAttachments = m.attachments.any { it.cid == null },
        size = (m.html ?: m.text ?: "").length.toLong(),
        // The demo's own headers, as desktop's demo reads them (demo.js mailSignals(m.headers)).
        signals = MailSignals.fromHeaders(m.headers),
    )

    override suspend fun fetchSource(path: String, uid: Long, background: Boolean): ByteArray {
        val m = read { it.box(path).find(uid) }
        // Like an IMAP server, the stored message as it was saved, Bcc included.
        return MimeComposer.compose(
            ComposedMail(
                from = m.from,
                to = m.to,
                cc = m.cc,
                bcc = m.bcc,
                subject = m.subject,
                text = m.text.orEmpty(),
                html = m.html,
                inReplyTo = m.inReplyTo,
                references = m.references.orEmpty(),
                date = m.date,
                messageId = m.messageId,
                attachments = m.attachments.map {
                    MailAttachment(it.filename, it.contentType, Base64.getMimeDecoder().decode(it.content), it.cid)
                },
                headers = m.headers.orEmpty(),
                keepBcc = true,
            ),
        )
    }

    override suspend fun setFlags(path: String, uids: List<Long>, unread: Boolean?, starred: Boolean?, answered: Boolean?) = withState(write = true) { s ->
        val box = s.box(path)
        uids.forEach { box.find(it) }
        val changed = box.messages.map { m ->
            if (m.uid !in uids) return@map m
            val set = m.flags.toMutableSet()
            if (unread == true) set.remove("\\Seen")
            if (unread == false) set.add("\\Seen")
            if (starred == true) set.add("\\Flagged")
            if (starred == false) set.remove("\\Flagged")
            if (answered == true) set.add("\\Answered")
            m.copy(flags = set.toList())
        }
        s.copy(boxes = s.boxes + (path to box.copy(messages = changed))) to Unit
    }

    /** Moved messages get new uids in the destination, so demo moves are always undoable (demo.js move). */
    override suspend fun move(path: String, uids: List<Long>, destination: String, expectUidValidity: String?): MoveOutcome = withState(write = true) { s ->
        val src = s.box(path)
        val dest = s.box(destination)
        if (expectUidValidity != null && src.uidValidity != expectUidValidity) throw UidValidityChanged()
        var next = s.nextUid
        val map = LinkedHashMap<Long, Long>()
        val moved = uids.map { uid -> src.find(uid).copy(uid = next).also { map[uid] = next++ } }
        val boxes = s.boxes +
            (path to src.copy(messages = src.messages.filter { it.uid !in uids })) +
            (destination to dest.copy(messages = (dest.messages + moved).sortedBy { it.uid }))
        s.copy(boxes = boxes, nextUid = next) to MoveOutcome(map, dest.uidValidity)
    }

    override suspend fun deleteForever(path: String, uids: List<Long>) = withState(write = true) { s ->
        val box = s.box(path)
        s.copy(boxes = s.boxes + (path to box.copy(messages = box.messages.filter { it.uid !in uids }))) to Unit
    }

    override suspend fun append(path: String, raw: ByteArray, flags: List<String>): Long? {
        val parsed = MimeParser.parse(raw)
        return withState(write = true) { s ->
            val box = s.box(path)
            val m = DemoMessage(
                uid = s.nextUid,
                from = parsed.from.firstOrNull() ?: Address("", Account.DEMO_EMAIL),
                to = parsed.to,
                cc = parsed.cc,
                bcc = parsed.bcc,
                subject = parsed.subject.orEmpty(),
                date = parsed.date ?: clock.now(),
                html = parsed.html,
                text = parsed.text,
                inReplyTo = parsed.inReplyTo,
                references = parsed.references.ifEmpty { null },
                attachments = parsed.attachments.map {
                    DemoAttachment(
                        filename = it.filename ?: "attachment-${it.index + 1}",
                        contentType = it.contentType,
                        content = Base64.getEncoder().encodeToString(it.bytes),
                        cid = it.contentId?.let(MimeParser::cidKey),
                    )
                },
                messageId = parsed.messageId ?: "<demo-${clock.now()}@example.com>",
                flags = flags,
            )
            s.copy(boxes = s.boxes + (path to box.copy(messages = box.messages + m)), nextUid = s.nextUid + 1) to m.uid
        }
    }

    override suspend fun send(mail: ComposedMail): ByteArray {
        val raw = MimeComposer.compose(mail)
        // Mail addressed to the demo account itself lands in its inbox, like a real round trip.
        if (mail.recipients.any { it.address.trim().equals(Account.DEMO_EMAIL, ignoreCase = true) }) append("INBOX", raw, emptyList())
        return raw
    }

    override fun savesSentCopy(): Boolean = false

    override suspend fun close() = Unit

    companion object {
        const val FILE_NAME = "demo-server.json"

        /**
         * Desktop's seed() (demo.js:44-334) at [now] in [zone]: the same folders, messages, uids,
         * Message-IDs, flags and threading, from [PreviewData].
         */
        fun seed(now: Long, zone: ZoneId = ZoneId.systemDefault()): DemoState {
            val seeded = PreviewData.seed(now, zone)
            val pdf = Base64.getEncoder().encodeToString(PreviewData.PDF)
            val boxes = linkedMapOf<String, DemoBox>()
            var counter = 1L
            for ((path, _) in PreviewData.FOLDERS) {
                val messages = seeded.filter { it.folder == path }.map { s ->
                    DemoMessage(
                        uid = s.uid,
                        from = s.from,
                        to = s.to,
                        cc = s.cc,
                        subject = s.subject,
                        date = s.date,
                        html = s.html,
                        text = s.text,
                        attachments = s.attachments.map { DemoAttachment(it, "application/pdf", pdf) },
                        headers = if (s.html?.contains("example.com/unsubscribe") == true) {
                            mapOf("List-Unsubscribe" to "<https://example.com/unsubscribe>", "List-Unsubscribe-Post" to "List-Unsubscribe=One-Click")
                        } else {
                            null
                        },
                        messageId = s.messageId,
                        inReplyTo = s.inReplyTo,
                        references = s.references.ifEmpty { null },
                        flags = buildList {
                            if (!s.unread) add("\\Seen")
                            if (s.starred) add("\\Flagged")
                            if (path == "Drafts") add("\\Draft")
                        },
                    )
                }
                counter += messages.size
                boxes[path] = DemoBox(uidValidity = "1", uidNext = counter, messages = messages)
            }
            return DemoState(boxes = boxes, nextUid = counter)
        }
    }
}
