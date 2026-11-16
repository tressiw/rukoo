package nl.bvdm.rukoo.mail

import java.io.File
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.async
import kotlinx.coroutines.cancel
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import nl.bvdm.rukoo.mail.compose.toOutgoing
import nl.bvdm.rukoo.core.Change
import nl.bvdm.rukoo.mail.model.Account
import nl.bvdm.rukoo.mail.model.AccountPatch
import nl.bvdm.rukoo.mail.model.Address
import nl.bvdm.rukoo.mail.model.AuthKind
import nl.bvdm.rukoo.mail.model.DeleteOutcome
import nl.bvdm.rukoo.mail.model.ImapAccountInput
import nl.bvdm.rukoo.mail.model.ImportedAccount
import nl.bvdm.rukoo.mail.model.MailView
import nl.bvdm.rukoo.mail.model.MessageIds
import nl.bvdm.rukoo.mail.model.MessageSummary
import nl.bvdm.rukoo.mail.model.OutgoingMessage
import nl.bvdm.rukoo.mail.model.ReplyKind
import nl.bvdm.rukoo.mail.model.ServerConfig
import nl.bvdm.rukoo.mail.model.SortOrder
import nl.bvdm.rukoo.mail.model.ViewKind
import okhttp3.OkHttpClient
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder

class EngineTest {
    @get:Rule
    val tmp = TemporaryFolder()

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private val newMail = mutableListOf<List<MessageSummary>>()

    @After
    fun stop() = scope.cancel()

    private fun engine(server: FakeServer, dir: File = tmp.root) = RukooMailEngine(
        dir = dir,
        attachmentsDir = File(dir, "attachments"),
        scope = scope,
        cipher = FakeCipher,
        strings = TestStrings,
        http = OkHttpClient(),
        sessionFactory = { _, _ -> server.session() },
        saveDelayMs = 5,
        onNewMail = { synchronized(newMail) { newMail += it } },
    )

    private val input = ImapAccountInput(
        email = "me@example.com",
        name = "Me",
        password = "hunter2",
        imap = ServerConfig("imap.example.com", 993),
        smtp = ServerConfig("smtp.example.com", 465),
    )

    private suspend fun RukooMailEngine.inbox(): List<MessageSummary> = listMessages(MailView(ViewKind.INBOX))

    private fun server(folders: List<nl.bvdm.rukoo.mail.session.FolderRecord> = FakeServer.STANDARD, inbox: Int = 3) = FakeServer(folders).apply {
        for (uid in 1L..inbox) add("INBOX", ServerMessage(uid, "Message $uid", date = 1_700_000_000_000 + uid * 1000))
    }

    private suspend fun added(engine: RukooMailEngine): Account {
        val acc = engine.addImapAccount(input)
        engine.refresh(acc.id)
        return acc
    }

    @Test
    fun `adding verifies first, stores the password encrypted and syncs the inbox`() = runBlocking {
        val server = server()
        server.failNext = "verify" to IllegalStateException("AUTHENTICATIONFAILED Invalid credentials")
        val engine = engine(server)
        try {
            engine.addImapAccount(input)
            fail()
        } catch (e: MailException) {
            assertEquals("Sign-in failed. Check your email address and password or app password.", e.message)
        }
        assertTrue(engine.accounts.value.isEmpty())

        val acc = added(engine)
        assertEquals(AuthKind.PASSWORD, acc.auth)
        assertEquals("me@example.com", acc.imap!!.user)
        assertEquals(listOf("Message 3", "Message 2", "Message 1"), engine.inbox().map { it.subject })
        assertEquals(MessageIds.encode(acc.id, "INBOX", 3), engine.inbox().first().id)
        val secrets = File(tmp.root, "secrets.json").readText()
        assertFalse(secrets.contains("hunter2"))
        assertTrue(secrets.contains(FakeCipher.encrypt("hunter2")))
        try {
            engine.addImapAccount(input)
            fail()
        } catch (e: MailException) {
            assertEquals("This account has already been added.", e.message)
        }
        try {
            engine.addImapAccount(input.copy(email = "not an address"))
            fail()
        } catch (e: MailException) {
            assertEquals("Enter a valid email address.", e.message)
        }
    }

    @Test
    fun `later syncs fetch only new uids, refresh flags and report new mail`() = runBlocking {
        val server = server()
        val engine = engine(server)
        val acc = added(engine)
        assertEquals(listOf(1L, 2L, 3L), server.fullyFetched.toList())
        assertTrue("the first sync announces nothing", newMail.isEmpty())

        server.fullyFetched.clear()
        server.add("INBOX", ServerMessage(4, "Fresh"))
        server.box("INBOX").messages.replaceAll { if (it.uid == 1L) it.copy(flags = setOf("\\Seen", "\\Flagged")) else it }
        engine.refresh(acc.id)
        assertEquals(listOf(4L), server.fullyFetched.toList())
        val one = engine.inbox().first { it.uid == 1L }
        assertFalse(one.unread)
        assertTrue(one.starred)
        assertEquals(listOf("Fresh"), newMail.single().map { it.subject })
        assertEquals(3, engine.unreadCount.first { it == 3 })
    }

    @Test
    fun `the inbox window keeps the newest 300 and a rebuilt folder starts over`() = runBlocking {
        val server = server(inbox = 350)
        val engine = engine(server)
        val acc = added(engine)
        val inbox = engine.inbox()
        assertEquals(300, inbox.size)
        assertEquals(51L, inbox.minOf { it.uid })

        engine.rawSource(inbox.first().id)
        val bodies = File(tmp.root, "bodies/${acc.id}")
        val before = bodies.listFiles()!!.map { it.name }
        assertTrue(before.isNotEmpty())

        server.fullyFetched.clear()
        server.box("INBOX").uidValidity = "2"
        engine.refresh(acc.id)
        assertEquals(300, server.fullyFetched.size)
        // Bodies under the old UIDVALIDITY are gone; a prefetch may already be filling the folder again.
        assertTrue(before.none { File(bodies, it).exists() })
    }

    @Test
    fun `a folder that fails to sync leaves the others current, and the account shows the error`() = runBlocking {
        val server = server()
        val engine = engine(server)
        val acc = added(engine)
        val synced = withTimeout(5_000) { engine.sync.first { acc.id in it.lastSync } }.lastSync.getValue(acc.id)
        server.add("INBOX", ServerMessage(4, "Fresh"))
        server.add("Archive", ServerMessage(40, "Filed"))
        // Sent is the second folder a sync goes through; Drafts, Trash and Archive come after it.
        server.failingFolders["Sent"] = IllegalStateException("NO Sent is unavailable")
        try {
            engine.refresh(acc.id)
            fail()
        } catch (e: MailException) {
            assertEquals("NO Sent is unavailable", e.message)
        }
        assertTrue(engine.inbox().any { it.subject == "Fresh" })
        assertEquals(listOf("Filed"), engine.listMessages(MailView.folder(acc.id, "Archive")).map { it.subject })
        assertEquals(listOf("Fresh"), newMail.single().map { it.subject })
        val failed = withTimeout(5_000) { engine.sync.first { it.errors[acc.id] != null } }
        assertEquals("NO Sent is unavailable", failed.errors[acc.id])
        assertEquals("not a sync that got through every folder", synced, failed.lastSync[acc.id])

        server.failingFolders.clear()
        engine.refresh(acc.id)
        val fixed = withTimeout(5_000) { engine.sync.first { acc.id !in it.errors } }
        assertTrue(fixed.lastSync.getValue(acc.id) >= synced)
    }

    @Test
    fun `unread inbox mail older than the cached window counts in Inbox, Unread, the badge and the folder`() = runBlocking {
        // 305 messages: the cache keeps the newest 300 (6..305), so 1..5 are older. Of those 1, 3 and 5 are unread; in
        // the window only 300 is.
        val server = server(inbox = 0)
        val unread = setOf(1L, 3L, 5L, 300L)
        for (uid in 1L..305) server.add("INBOX", ServerMessage(uid, "Message $uid", flags = if (uid in unread) emptySet() else setOf("\\Seen")))
        val dir = tmp.newFolder("older")
        val engine = engine(server, dir)
        val acc = added(engine)
        assertEquals("only the inbox asks", listOf("INBOX"), server.olderAsked.distinct())
        assertEquals(300, engine.inbox().size)
        assertEquals(1, engine.inbox().count { it.unread })

        val counts = engine.counts(acc.id).first { it.olderUnread == 3 }
        assertEquals(4, counts.views[ViewKind.INBOX])
        assertEquals(4, counts.views[ViewKind.UNREAD])
        assertEquals("VIP picks its mail by sender, so the older mail stays out", 0, counts.views[ViewKind.VIP])
        assertEquals(4, counts.accounts[acc.id])
        assertEquals(4, counts.all)
        assertEquals(4, counts.folders["INBOX"])
        assertEquals(0, counts.folders["Sent"])
        assertEquals("what the inbox's note says Rukoo keeps", 300, counts.inboxLimit)
        assertEquals(4, engine.counts(null).first { it.olderUnread == 3 }.views[ViewKind.INBOX])
        assertEquals(4, engine.unreadCount.first { it == 4 })

        // Reading the cached one counts at once; the older ones stay until the next sync says otherwise.
        engine.setFlags(listOf(engine.inbox().first { it.unread }.id), unread = false)
        assertEquals(3, engine.counts(acc.id).first { it.views[ViewKind.INBOX] == 3 }.olderUnread)

        // The count is kept with the cache, so a restart shows it before the first sync.
        engine.flush()
        val again = engine(server, dir)
        assertEquals(3, again.counts(acc.id).first { it.olderUnread > 0 }.views[ViewKind.INBOX])

        // A server that turns the search down: the count is unknown, and nothing is added.
        server.olderUnseenRefused = true
        again.refresh(acc.id)
        assertEquals(0, again.counts(acc.id).first { it.olderUnread == 0 }.views[ViewKind.INBOX])
    }

    @Test
    fun `a sync that fails because the engine closed opens no new session for the next folder`() = runBlocking {
        val server = server()
        val engine = engine(server)
        val acc = added(engine)
        assertEquals(1, server.sessionsMade)
        server.syncDelayMs = 200
        val run = async { runCatching { engine.refresh(acc.id) } }
        withTimeout(5_000) { engine.sync.first { acc.id in it.running } }
        engine.close()
        assertTrue(run.await().isFailure)
        assertEquals(1, server.sessionsMade)
    }

    @Test
    fun `a refused flag change is reverted`() = runBlocking {
        val server = server()
        val engine = engine(server)
        added(engine)
        val id = engine.inbox().first().id
        server.failNext = "setFlags" to IllegalStateException("NO STORE failed")
        try {
            engine.setFlags(listOf(id), unread = false)
            fail()
        } catch (e: MailException) {
            assertEquals("NO STORE failed", e.message)
        }
        assertTrue(engine.summary(id)!!.unread)
        engine.setFlags(listOf(id), unread = false, starred = true)
        assertFalse(engine.summary(id)!!.unread)
        assertEquals(setOf("\\Seen", "\\Flagged"), server.box("INBOX").messages.first { it.uid == 3L }.flags)
    }

    @Test
    fun `archive moves with an undo, and undo brings it back`() = runBlocking {
        val server = server()
        val engine = engine(server)
        val acc = added(engine)
        val id = engine.inbox().first().id
        val result = engine.archive(listOf(id))
        assertEquals("Archive", result.folder)
        val moved = result.moved.single()
        assertTrue(moved.undoable)
        assertEquals(MessageIds.encode(acc.id, "Archive", 1000), moved.newId)
        assertTrue(engine.canUndo(id))
        assertEquals(2, engine.inbox().size)
        assertEquals("Archive", engine.findMoved(acc.id, "<m3@example.com>", "INBOX")?.let { MessageIds.decode(it)?.folder })

        val undo = engine.undoMove(listOf(id))
        assertTrue(undo.failed.isEmpty())
        assertEquals(MessageIds.encode(acc.id, "INBOX", 1001), undo.restored.single())
        assertEquals(3, engine.inbox().size)
        assertFalse(engine.canUndo(id))
        assertEquals("This action can no longer be undone.", engine.undoMove(listOf(id)).failed.single().error)
    }

    @Test
    fun `without UIDPLUS a move can't be undone`() = runBlocking {
        val server = server().apply { uidplus = false }
        val engine = engine(server)
        added(engine)
        val id = engine.inbox().first().id
        val moved = engine.move(listOf(id), "Trash").moved.single()
        assertFalse(moved.undoable)
        assertNull(moved.newId)
        assertFalse(engine.canUndo(id))
    }

    @Test
    fun `a failed move puts the message back`() = runBlocking {
        val server = server()
        val engine = engine(server)
        added(engine)
        val id = engine.inbox().first().id
        server.failNext = "move" to java.net.SocketTimeoutException("read timed out")
        val result = engine.archive(listOf(id))
        assertEquals("The connection to the server timed out.", result.failed.single().error)
        assertNotNull(engine.summary(id))
    }

    @Test
    fun `delete moves to Trash, then asks before deleting for good`() = runBlocking {
        val server = server()
        val engine = engine(server)
        val acc = added(engine)
        val id = engine.inbox().first().id
        val trashed = engine.delete(listOf(id))
        assertEquals(DeleteOutcome.TRASHED, trashed.outcome)
        val inTrash = trashed.move.moved.single().newId!!
        engine.refresh(acc.id)
        assertEquals(DeleteOutcome.NEEDS_CONFIRM, engine.delete(listOf(inTrash)).outcome)
        assertEquals(1, server.box("Trash").messages.size)
        assertEquals(DeleteOutcome.DELETED, engine.delete(listOf(inTrash), force = true).outcome)
        assertTrue(server.box("Trash").messages.isEmpty())
        assertTrue(engine.listMessages(MailView(ViewKind.TRASH)).isEmpty())
    }

    @Test
    fun `Gmail archives to All Mail and hides its copies of mail that is elsewhere`() = runBlocking {
        val server = FakeServer(FakeServer.GMAIL)
        server.add("INBOX", ServerMessage(1, "In inbox", messageId = "<a@x>"))
        server.add("[Gmail]/All Mail", ServerMessage(1, "In inbox", messageId = "<a@x>"))
        server.add("[Gmail]/All Mail", ServerMessage(2, "Labelled", messageId = "<b@x>"))
        server.add("[Gmail]/All Mail", ServerMessage(3, "Archived", messageId = "<c@x>"))
        server.add("Label", ServerMessage(1, "Labelled", messageId = "<b@x>"))
        val engine = engine(server)
        val acc = added(engine)
        engine.openFolder(acc.id, "Label")
        assertEquals("[Gmail]/All Mail", engine.archiveFolder(acc.id))
        assertEquals(listOf("Archived", "Labelled"), engine.listMessages(MailView(ViewKind.ARCHIVE)).map { it.subject }.sorted())
        val everything = engine.listMessages(MailView(ViewKind.EVERYTHING))
        assertEquals(listOf("INBOX", "Label", "[Gmail]/All Mail"), everything.map { it.folder }.sorted())
        assertEquals("Archived", everything.single { it.folder == "[Gmail]/All Mail" }.subject)
        assertEquals(listOf("INBOX", "[Gmail]/All Mail"), engine.findByMessageId("A@X").map { it.folder })
        assertEquals(1, engine.folders.first { it[acc.id]?.size == 6 }.getValue(acc.id).count { it.path == "[Gmail]/Starred" })
    }

    @Test
    fun `views, counts, search, sort and contacts work like desktop`() = runBlocking {
        val server = server(inbox = 0)
        val sanne = Address("Sanne", "sanne@example.com")
        server.add("INBOX", ServerMessage(1, "Proposal", from = sanne, date = 3_000))
        server.add("INBOX", ServerMessage(2, "Newsletter", from = Address("Shop", "deals@shop.example"), date = 2_000, flags = setOf("\\Seen")))
        server.add("INBOX", ServerMessage(3, "Alerts", from = Address("", "no-reply@bank.example"), date = 1_000, flags = setOf("\\Flagged")))
        server.add("Sent", ServerMessage(4, "Re: Proposal", from = Address("Me", "me@example.com"), to = listOf(Address("Joris", "joris@example.com")), flags = setOf("\\Seen")))
        server.add("Trash", ServerMessage(5, "Old", flags = setOf("\\Flagged")))
        val engine = engine(server)
        val acc = added(engine)

        assertEquals(listOf("Alerts"), engine.listMessages(MailView(ViewKind.STARRED)).map { it.subject })
        assertEquals(listOf("Proposal"), engine.listMessages(MailView(ViewKind.INBOX), query = "SANNE").map { it.subject })
        assertEquals(listOf("Re: Proposal"), engine.listMessages(MailView(ViewKind.SENT), query = "joris").map { it.subject })
        assertEquals(listOf("Proposal", "Alerts", "Newsletter"), engine.listMessages(MailView(ViewKind.INBOX), sort = SortOrder.UNREAD).map { it.subject })
        assertEquals(listOf("Alerts", "Newsletter", "Proposal"), engine.listMessages(MailView(ViewKind.INBOX), sort = SortOrder.DATE_ASC).map { it.subject })
        assertEquals(listOf("Alerts", "Proposal", "Newsletter"), engine.listMessages(MailView(ViewKind.INBOX), sort = SortOrder.SENDER).map { it.subject })

        val counts = engine.counts(acc.id).first()
        assertEquals(2, counts.all)
        assertEquals(2, counts.views[ViewKind.INBOX])
        assertEquals(1, counts.views[ViewKind.STARRED])
        assertEquals(1, counts.views[ViewKind.SENT])
        assertEquals(2, counts.folders["INBOX"])

        // Joris (written to, weight 3) ranks above Sanne (wrote to us); no-reply and own addresses are left out.
        assertEquals(listOf("joris@example.com", "sanne@example.com", "deals@shop.example", "sender@example.com"), engine.contacts().map { it.address })

        assertTrue(engine.toggleVip("SANNE@example.com"))
        assertEquals(listOf("Proposal"), engine.listMessages(MailView(ViewKind.VIP)).map { it.subject })
        assertTrue(engine.inbox().first { it.subject == "Proposal" }.vip)
        engine.markSpam(listOf(engine.inbox().first { it.subject == "Newsletter" }.id))
        assertEquals(listOf("Proposal", "Alerts"), engine.inbox().map { it.subject })
        assertEquals(listOf("deals@shop.example"), engine.prefs.value.spam)
        assertEquals(2, engine.markAllRead(MailView(ViewKind.INBOX)))
        assertEquals(0, engine.unreadCount.first { it == 0 })
    }

    @Test
    fun `replying sends threaded mail, files the Sent copy and marks the original answered`() = runBlocking {
        val server = server()
        val engine = engine(server)
        val acc = added(engine)
        val original = engine.inbox().first()
        val fields = engine.replyFields(original.id, ReplyKind.REPLY)
        assertEquals(listOf(Address("Sender", "sender@example.com")), fields.to)
        val outgoing = fields.toComposerState().copy(body = "Thanks!").toOutgoing(acc.id, Address("Me", "me@example.com"))
        engine.send(outgoing)

        val sent = server.sent.single()
        assertEquals("<m3@example.com>", sent.inReplyTo)
        assertEquals(listOf("<m3@example.com>"), sent.references)
        assertEquals("Re: Message 3", sent.subject)
        assertTrue(sent.html!!.contains("data-rukoo-quote"))
        assertTrue(sent.text.startsWith("Thanks!\n\n-------- Original message --------"))
        assertEquals("Sent", server.appended.single().first)
        assertTrue(engine.summary(original.id)!!.answered)
        assertTrue("\\Answered" in server.box("INBOX").messages.first { it.uid == 3L }.flags)

        try {
            engine.send(OutgoingMessage(acc.id, Address("", "me@example.com"), subject = "Nobody"))
            fail()
        } catch (e: MailException) {
            assertEquals("Add at least one recipient.", e.message)
        }
        try {
            engine.send(OutgoingMessage(acc.id, Address("", "someone@else.example"), to = listOf(Address("", "a@b.example"))))
            fail()
        } catch (e: MailException) {
            assertEquals("You cannot send as someone@else.example from this account.", e.message)
        }
        try {
            engine.send(OutgoingMessage(acc.id, Address("", "me@example.com"), to = listOf(Address("", "broken"))))
            fail()
        } catch (e: MailException) {
            assertEquals("Invalid email address: broken", e.message)
        }
    }

    @Test
    fun `sent mail carries the account's HTML signature, or the general one, and none when left out`() = runBlocking {
        val server = server()
        val engine = engine(server)
        val acc = added(engine)
        val original = engine.inbox().first()
        val logo = "<b>Bart</b><br><img src=\"https://bvdm.ai/logo.png\" width=\"80\">"
        engine.updateAccount(acc.id, AccountPatch(signatureHtml = Change(logo)))
        val fields = engine.replyFields(original.id, ReplyKind.REPLY)
        // The composer's text never holds the signature; an agent's draft replaces only the text.
        assertEquals("", fields.body)
        engine.send(fields.toComposerState().copy(body = "Thanks!").toOutgoing(acc.id, Address("Me", "me@example.com")))
        val html = server.sent.last().html!!
        assertTrue(html.contains("<div>Thanks!</div><div><br></div><div class=\"signature\"><b>Bart</b><br><img src=\"https://bvdm.ai/logo.png\" width=\"80\"></div>"))
        assertTrue(html.indexOf("class=\"signature\"") < html.indexOf("data-rukoo-quote"))

        // No signature of its own: the general one, as desktop's usesGeneralSignature.
        engine.updateAccount(acc.id, AccountPatch(signature = Change(null)))
        engine.updatePrefs { it.copy(signature = "", signatureHtml = "<i>Sent with Rukoo</i>") }
        engine.send(OutgoingMessage(acc.id, Address("Me", "me@example.com"), to = listOf(Address("", "x@example.com")), subject = "Hi", text = "Hello"))
        assertTrue(server.sent.last().html!!.contains("<div class=\"signature\"><i>Sent with Rukoo</i></div>"))
        assertTrue(server.sent.last().text.startsWith("Hello\n\nSent with Rukoo"))

        engine.send(OutgoingMessage(acc.id, Address("Me", "me@example.com"), to = listOf(Address("", "x@example.com")), subject = "Hi", text = "Hello", signature = false))
        assertFalse(server.sent.last().html!!.contains("class=\"signature\""))
    }

    @Test
    fun `a draft just saved reads back without a download`() = runBlocking {
        val server = server()
        val engine = engine(server)
        val acc = added(engine)
        val stored = engine.saveDraft(OutgoingMessage(acc.id, Address("", "me@example.com"), to = listOf(Address("", "x@example.com")), subject = "Plan", text = "Draft"))!!
        // The composer reads the new copy's attachments back right after saving; that must not need the network.
        server.failNext = "fetchSource" to java.io.IOException("offline")
        assertEquals("Draft", engine.fullMessage(stored).text)
    }

    @Test
    fun `sending an edited desktop draft removes it from Drafts`() = runBlocking {
        val server = server()
        val engine = engine(server)
        val acc = added(engine)
        val stored = engine.saveDraft(OutgoingMessage(acc.id, Address("", "me@example.com"), to = listOf(Address("", "x@example.com")), subject = "Plan", text = "Draft"))!!
        val state = engine.replyFields(stored, ReplyKind.DRAFT).toComposerState()
        assertEquals(stored, state.draftId)
        engine.send(state.copy(body = "Draft, finished").toOutgoing(acc.id, Address("", "me@example.com")))
        assertTrue(server.box("Drafts").messages.isEmpty())
        assertNull(engine.summary(stored))
        assertTrue(server.sent.last().text.startsWith("Draft, finished"))
    }

    @Test
    fun `drafts are stored on the server, replaced on the next save and found by their id`() = runBlocking {
        val server = server()
        val engine = engine(server)
        val acc = added(engine)
        val message = OutgoingMessage(acc.id, Address("", "me@example.com"), to = listOf(Address("", "x@example.com")), bcc = listOf(Address("", "hidden@example.com")), subject = "Plan", text = "Draft one")
        val first = engine.saveDraft(message)!!
        assertEquals("Drafts", MessageIds.decode(first)!!.folder)
        assertEquals(listOf("hidden@example.com"), engine.fullMessage(first).bcc.map { it.address })
        val second = engine.saveDraft(message.copy(text = "Draft two", draftId = first))!!
        assertEquals(1, server.box("Drafts").messages.size)
        assertNull(engine.summary(first))
        assertEquals("Draft two", engine.fullMessage(second).text)
        val reopened = engine.replyFields(second, ReplyKind.DRAFT)
        assertEquals("Draft two", reopened.body)
        assertEquals(second, reopened.draftId)
        engine.discardDraft(second)
        assertTrue(server.box("Drafts").messages.isEmpty())

        server.uidplus = false
        val found = engine.saveDraft(message.copy(text = "Without UIDPLUS"))
        assertNotNull("found by its Message-ID after the Drafts sync", found)
    }

    @Test
    fun `everything is read back after a restart, and removing an account deletes its files`() = runBlocking {
        val server = server()
        val dir = tmp.newFolder("profile")
        val engine = engine(server, dir)
        val acc = added(engine)
        engine.updatePrefs { it.copy(vips = listOf("Sender@Example.com"), signature = "Groet") }
        engine.updateAccount(acc.id, nl.bvdm.rukoo.mail.model.AccountPatch(name = "Bart", signature = nl.bvdm.rukoo.core.Change("Bart\nbvdm.ai")))
        engine.flush()

        val again = engine(server, dir)
        assertEquals(listOf("Bart"), again.accounts.first { it.isNotEmpty() }.map { it.name })
        assertEquals(3, again.listMessages(MailView(ViewKind.INBOX)).size)
        assertEquals(listOf("sender@example.com"), again.prefs.value.vips)
        assertEquals("Bart\nbvdm.ai", again.signatureFor(acc.id)?.text)

        again.removeAccount(acc.id)
        assertFalse(File(dir, "cache/${acc.id}.json").exists())
        assertFalse(File(dir, "secrets.json").readText().contains(acc.id))
        assertTrue(again.accounts.value.isEmpty())
    }

    @Test
    fun `imported accounts skip duplicates and secrets that didn't decrypt`() = runBlocking {
        val server = server()
        val engine = engine(server)
        added(engine)
        val imported = engine.importAccounts(
            listOf(
                ImportedAccount(id = "a1", email = "me@example.com", password = "x"),
                ImportedAccount(id = "a2", email = "work@example.com", password = "pw", imap = ServerConfig("imap.work.example", 993), smtp = ServerConfig("smtp.work.example", 587, false)),
                ImportedAccount(id = "a3", email = "broken@example.com", secretError = "decrypt-failed"),
                ImportedAccount(id = "a4", email = "g@gmail.com", provider = "google", auth = AuthKind.OAUTH2, refreshToken = "rt", scopes = listOf("b", "a", "a")),
            ),
            googleClient = null,
        )
        assertEquals(listOf("work@example.com", "g@gmail.com"), imported.map { it.email })
        assertEquals("a2", imported[0].id)
        assertEquals("work@example.com", imported[0].imap!!.user)
        assertEquals("imap.gmail.com", imported[1].imap!!.host)
        assertEquals(listOf("a", "b"), imported[1].scopes)
        assertTrue(File(tmp.root, "secrets.json").readText().contains(FakeCipher.encrypt("rt")))
    }

    @Test
    fun `unreadable account data stops the engine instead of being written over`() = runBlocking {
        val dir = tmp.newFolder("broken")
        File(dir, "accounts.json").mkdirs()
        File(dir, "secrets.json").writeText("{\"old\":\"enc:x\"}")
        val engine = engine(server(), dir)
        try {
            engine.addDemoAccount()
            fail()
        } catch (e: MailException) {
            assertEquals("The mail data on this phone could not be read. Restart Rukoo Mail to try again.", e.message)
        }
        assertFalse(engine.loaded.value)
        assertEquals("{\"old\":\"enc:x\"}", File(dir, "secrets.json").readText())
    }

    @Test
    fun `a change runs to the end when its caller is cancelled`() = runBlocking {
        val server = server()
        val engine = engine(server)
        added(engine)
        val id = engine.inbox().first().id
        server.moveDelayMs = 300
        val caller = launch(Dispatchers.Default) { engine.archive(listOf(id)) }
        delay(100)
        caller.cancelAndJoin()
        // The server moved it, and the cache agrees once the change finishes.
        withTimeout(5_000) { while (engine.canUndo(id).not()) delay(20) }
        assertEquals(1, server.box("Archive").messages.size)
        assertNull(engine.summary(id))
        assertEquals(2, engine.inbox().size)
    }

    @Test
    fun `a failed batch flag change puts back each message's own value`() = runBlocking {
        val server = server()
        server.box("INBOX").messages.replaceAll { if (it.uid == 1L) it.copy(flags = setOf("\\Seen")) else it }
        val engine = engine(server)
        added(engine)
        val read = engine.inbox().first { it.uid == 1L }
        val unread = engine.inbox().first { it.uid == 2L }
        server.failNext = "setFlags" to IllegalStateException("NO")
        try {
            engine.setFlags(listOf(read.id, unread.id), unread = false)
            fail()
        } catch (e: MailException) {
            assertEquals("NO", e.message)
        }
        assertFalse(engine.summary(read.id)!!.unread)
        assertTrue(engine.summary(unread.id)!!.unread)
    }

    @Test
    fun `a failed flag change doesn't undo a later one that succeeds`() = runBlocking {
        val server = server()
        val engine = engine(server)
        added(engine)
        val id = engine.inbox().first().id
        server.setFlagsDelayMs = 200
        server.failNext = "setFlags" to IllegalStateException("NO")
        val first = async(Dispatchers.Default) { runCatching { engine.setFlags(listOf(id), starred = true) } }
        delay(50)
        val second = async(Dispatchers.Default) { runCatching { engine.setFlags(listOf(id), starred = true) } }
        assertTrue(first.await().isFailure)
        assertTrue(second.await().isSuccess)
        assertTrue(engine.summary(id)!!.starred)
        assertTrue("\\Flagged" in server.box("INBOX").messages.first { it.uid == 3L }.flags)
    }

    @Test
    fun `a removed account's cache isn't written back by a pending save`() = runBlocking {
        val server = server()
        val dir = tmp.newFolder("removal")
        val engine = RukooMailEngine(
            dir, File(dir, "attachments"), scope, FakeCipher, TestStrings, OkHttpClient(),
            sessionFactory = { _, _ -> server.session() }, saveDelayMs = 200,
        )
        val acc = added(engine)
        engine.setFlags(listOf(engine.inbox().first().id), starred = true)
        engine.removeAccount(acc.id)
        delay(400)
        engine.flush()
        assertFalse(File(dir, "cache/${acc.id}.json").exists())
    }

    @Test
    fun `attachments with the same name are kept apart`() = runBlocking {
        val server = server(inbox = 0)
        val raw = nl.bvdm.rukoo.mail.mime.MimeComposer.compose(
            nl.bvdm.rukoo.mail.mime.ComposedMail(
                from = Address("", "billing@example.com"), to = listOf(Address("", "me@example.com")), subject = "Two invoices", text = "x",
                date = 1, messageId = "<two@x>",
                attachments = listOf(
                    nl.bvdm.rukoo.mail.mime.MailAttachment("invoice.pdf", "application/pdf", "first".toByteArray()),
                    nl.bvdm.rukoo.mail.mime.MailAttachment("invoice.pdf", "application/pdf", "second".toByteArray()),
                ),
            ),
        )
        server.add("INBOX", ServerMessage(1, "Two invoices", messageId = "<two@x>", raw = raw))
        val engine = engine(server)
        added(engine)
        val id = engine.inbox().single().id
        val (a, b) = engine.fullMessage(id).attachments
        val first = engine.attachmentFile(id, a.index)
        val second = engine.attachmentFile(id, b.index)
        assertEquals("invoice.pdf", first.name)
        assertFalse(first == second)
        assertEquals("first", first.readText())
        assertEquals("second", second.readText())
    }

    @Test
    fun `a folder is created once and shows in the list`() = runBlocking {
        val server = server()
        val engine = engine(server)
        val acc = added(engine)
        try {
            engine.createFolder(acc.id, "a/b")
            fail()
        } catch (e: MailException) {
            assertEquals("A folder name cannot contain / \\ % or *.", e.message)
        }
        try {
            engine.createFolder(acc.id, "projects")
            fail()
        } catch (e: MailException) {
            assertEquals("A folder with that name already exists.", e.message)
        }
        assertEquals("Receipts", engine.createFolder(acc.id, " Receipts "))
    }
}
