package nl.bvdm.rukoo.mail

import java.io.File
import java.time.ZoneId
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.runBlocking
import nl.bvdm.rukoo.core.Clock
import nl.bvdm.rukoo.core.JsonFiles
import nl.bvdm.rukoo.mail.compose.toOutgoing
import nl.bvdm.rukoo.mail.demo.DemoSession
import nl.bvdm.rukoo.mail.demo.DemoState
import nl.bvdm.rukoo.mail.model.Address
import nl.bvdm.rukoo.mail.model.AuthKind
import nl.bvdm.rukoo.mail.model.MailView
import nl.bvdm.rukoo.mail.model.MessageSummary
import nl.bvdm.rukoo.mail.model.OutgoingMessage
import nl.bvdm.rukoo.mail.model.ReplyKind
import nl.bvdm.rukoo.mail.model.UnsubscribeResult
import nl.bvdm.rukoo.mail.model.ViewKind
import okhttp3.OkHttpClient
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder

class DemoTest {
    @get:Rule
    val tmp = TemporaryFolder()

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private val zone = ZoneId.of("Europe/Amsterdam")

    // 22 October 2026, 16:30 in Amsterdam: after 15:00, so today's mail keeps its own times.
    private val now = java.time.ZonedDateTime.of(2026, 10, 22, 16, 30, 0, 0, zone).toInstant().toEpochMilli()
    private val newMail = mutableListOf<MessageSummary>()

    @After
    fun stop() = scope.cancel()

    private fun engine(dir: File = tmp.root) = RukooMailEngine(
        dir = dir,
        attachmentsDir = File(dir, "attachments"),
        scope = scope,
        cipher = FakeCipher,
        strings = TestStrings,
        http = OkHttpClient(),
        clock = Clock { now },
        zone = { zone },
        saveDelayMs = 5,
        onNewMail = { synchronized(newMail) { newMail += it } },
    )

    @Test
    fun `the seed matches desktop's uids, Message-IDs, flags and threading`() {
        val state = DemoSession.seed(now, zone)
        assertEquals(26L, state.nextUid)
        assertEquals(listOf("INBOX", "Sent", "Drafts", "Trash", "Junk", "Archive", "Invoices", "Travel"), state.boxes.keys.toList())
        val inbox = state.boxes.getValue("INBOX").messages
        assertEquals((1L..16L).toList(), inbox.map { it.uid })
        assertEquals("<demo-2@example.com>", inbox[0].messageId)
        assertEquals(listOf(1L, 2L, 3L, 4L, 5L, 10L, 12L), inbox.filter { "\\Seen" !in it.flags }.map { it.uid })
        assertEquals(listOf(11L, 15L), inbox.filter { "\\Flagged" in it.flags }.map { it.uid })
        assertEquals(listOf(1L, 2L, 10L, 11L), inbox.filter { it.headers?.get("List-Unsubscribe") == "<https://example.com/unsubscribe>" }.map { it.uid })
        assertEquals("<demo-19@example.com>", inbox[14].inReplyTo)
        val sent = state.boxes.getValue("Sent").messages
        assertEquals(listOf(17L, 18L, 19L), sent.map { it.uid })
        assertEquals(listOf("<demo-13@example.com>"), sent[0].references)
        val drafts = state.boxes.getValue("Drafts").messages
        assertTrue(drafts.all { "\\Draft" in it.flags && "\\Seen" in it.flags })
        assertEquals(emptyList<Address>(), drafts[1].to)
        assertEquals("<demo-26@example.com>", state.boxes.getValue("Travel").messages.single().messageId)
        assertEquals(1, state.boxes.getValue("Invoices").messages.single().attachments.size)
        // Today's mail at its own time after 15:00.
        assertEquals(java.time.ZonedDateTime.of(2026, 10, 22, 14, 45, 0, 0, zone).toInstant().toEpochMilli(), inbox[0].date)
    }

    @Test
    fun `the demo account syncs, opens real MIME and unsubscribes without network`() = runBlocking {
        val engine = engine()
        val acc = engine.addDemoAccount()
        assertEquals(AuthKind.NONE, acc.auth)
        assertEquals("Demo User", acc.name)
        engine.refresh(acc.id)
        val inbox = engine.listMessages(MailView(ViewKind.INBOX))
        assertEquals(16, inbox.size)
        assertEquals("Traffic fines in 2027: what you will pay", inbox.first().subject)
        assertEquals(7, inbox.count { it.unread })
        assertEquals(listOf("5 more exposures ready to review", "Re: Dinner on Saturday"), engine.listMessages(MailView(ViewKind.STARRED)).map { it.subject })
        assertEquals("Archive", engine.archiveFolder(acc.id))
        assertEquals(acc, engine.addDemoAccount())

        val newsletter = engine.fullMessage(inbox.first().id)
        assertTrue(newsletter.isHtml)
        assertTrue(newsletter.html.contains("ANWB"))
        assertTrue(newsletter.text.contains("Driving in low sun"))
        assertEquals("https://example.com/unsubscribe", newsletter.unsubscribe?.url)
        assertTrue(newsletter.unsubscribe!!.oneClick)
        assertEquals(UnsubscribeResult.Done, engine.unsubscribe(inbox.first().id))

        val contract = inbox.first { it.subject == "Here is another copy of your contract" }
        assertTrue(contract.hasAttachments)
        val full = engine.fullMessage(contract.id)
        assertEquals("Contract-confirmation.pdf", full.attachments.single().filename)
        assertTrue(String(engine.attachment(contract.id, full.attachments.single().index).bytes).startsWith("%PDF-1.4"))
        val file = engine.attachmentFile(contract.id, full.attachments.single().index)
        assertEquals("Contract-confirmation.pdf", file.name)
        assertTrue(file.path.contains("attachments"))
        assertEquals(UnsubscribeResult.Unavailable, engine.unsubscribe(contract.id))
    }

    @Test
    fun `replying to the call fills the composer like desktop`() = runBlocking {
        val engine = engine()
        val acc = engine.addDemoAccount()
        engine.refresh(acc.id)
        val call = engine.listMessages(MailView(ViewKind.INBOX)).first { it.subject == "Call on Thursday" }
        val all = engine.replyFields(call.id, ReplyKind.REPLY_ALL)
        assertEquals(listOf("sanne@example.com"), all.to.map { it.address })
        assertEquals(listOf("joris@example.com"), all.cc.map { it.address })
        assertEquals("Re: Call on Thursday", all.subject)
        assertEquals("<demo-13@example.com>", all.inReplyTo)
        assertTrue(all.quote.contains("From: Sanne de Vries &lt;sanne@example.com&gt;"))
        val fwd = engine.replyFields(call.id, ReplyKind.FORWARD)
        assertEquals(listOf("Proposal-v3.pdf"), fwd.attachments.map { it.filename })
    }

    @Test
    fun `the demo server counts unread mail older than a sync's window, as a real one does`() = runBlocking {
        val demo = DemoSession(File(tmp.newFolder("server"), DemoSession.FILE_NAME), TestStrings, Clock { now }, zone)
        // The seed's inbox holds 1..16; a window of ten leaves 1..6 older, of which 1..5 are unread.
        val res = demo.syncFolder("INBOX", 10, emptyMap(), null, olderUnseen = true)
        assertEquals((7L..16L).toList(), res.messages.map { it.uid })
        assertEquals(5, res.olderUnseen)
        assertEquals(0, demo.syncFolder("INBOX", 300, emptyMap(), null, olderUnseen = true).olderUnseen)
        assertEquals(null, demo.syncFolder("INBOX", 10, emptyMap(), null).olderUnseen)
    }

    @Test
    fun `mail to the demo address arrives as new mail, and moves persist in the server file`() = runBlocking {
        val dir = tmp.newFolder("demo")
        val engine = engine(dir)
        val acc = engine.addDemoAccount()
        engine.refresh(acc.id)
        val call = engine.listMessages(MailView(ViewKind.INBOX)).first { it.subject == "Call on Thursday" }
        val fields = engine.replyFields(call.id, ReplyKind.FORWARD)
        engine.send(
            fields.toComposerState().copy(to = listOf(Address("", "demo@example.com")), body = "For myself").toOutgoing(acc.id, Address("", "demo@example.com")),
        )
        engine.refresh(acc.id)
        val arrived = newMail.single()
        assertEquals("Fwd: Call on Thursday", arrived.subject)
        assertTrue(arrived.unread)
        assertTrue(arrived.hasAttachments)
        assertEquals(1, engine.listMessages(MailView(ViewKind.SENT)).count { it.subject == "Fwd: Call on Thursday" })

        val bol = engine.listMessages(MailView(ViewKind.INBOX)).first { it.subject == "Your parcel is on its way" }
        val moved = engine.archive(listOf(bol.id)).moved.single()
        assertTrue(moved.undoable)
        engine.setFlags(listOf(moved.newId!!), starred = true)
        val state = JsonFiles.read(File(dir, DemoSession.FILE_NAME), DemoState.serializer(), DemoState())
        val archived = state.boxes.getValue("Archive").messages.single()
        assertEquals("Your parcel is on its way", archived.subject)
        assertTrue("\\Flagged" in archived.flags)

        engine.send(OutgoingMessage(acc.id, Address("", "demo@example.com"), to = listOf(Address("", "someone@example.com")), subject = "Away", text = "x"))
        assertEquals(1, newMail.size)
    }
}
