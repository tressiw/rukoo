package nl.bvdm.rukoo.mail.session

import kotlinx.coroutines.runBlocking
import nl.bvdm.rukoo.mail.TestStrings
import nl.bvdm.rukoo.mail.model.Account
import nl.bvdm.rukoo.mail.model.AccountType
import nl.bvdm.rukoo.mail.model.AuthKind
import nl.bvdm.rukoo.mail.model.ServerConfig
import nl.bvdm.rukoo.mail.session.FakeImap.Msg
import org.eclipse.angus.mail.iap.Response
import org.eclipse.angus.mail.imap.protocol.IMAPResponse
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Test

/**
 * The inbox sync's count of unread mail older than its window (desktop's unseenBelow in imap.js and its tests): one
 * search over the uids below the window, answered by a small scripted IMAP server (FakeImap) on a local socket.
 */
class ImapOlderUnseenTest {
    private val servers = mutableListOf<FakeImap>()
    private val sessions = mutableListOf<ImapSession>()

    @After
    fun tearDown() = runBlocking {
        sessions.forEach { it.close() }
        servers.forEach { it.close() }
    }

    private fun session(server: FakeImap): ImapSession {
        val config = ServerConfig("127.0.0.1", server.port, secure = false, user = "me@example.com")
        val account = Account("acc", AccountType.IMAP, "other", AuthKind.PASSWORD, "me@example.com", "Me", imap = config, smtp = config)
        return ImapSession(account, Credentials.Password("secret"), TestStrings, connectTimeoutMs = 5_000, socketTimeoutMs = 10_000).also { sessions += it }
    }

    // Five messages, of which 10 and 12 are unread below a window of the newest two (13 and 14).
    private fun inbox(capabilities: String = "IMAP4rev1") = FakeImap(
        listOf(
            Msg(10, "old unread"),
            Msg(11, "old read", flags = "\\Seen"),
            Msg(12, "old unread too"),
            Msg(13, "new unread"),
            Msg(14, "new read", flags = "\\Seen"),
        ),
        capabilities,
    ).also { servers += it }

    private fun FakeImap.unseenSearches() = commands.filter { it.startsWith("UID SEARCH") && it.endsWith("UNSEEN") }

    @Test
    fun `the uids below the window are searched for unread mail and counted`() = runBlocking {
        val server = inbox()
        val res = session(server).syncFolder("INBOX", 2, emptyMap(), null, olderUnseen = true)
        assertEquals(listOf(13L, 14L), res.messages.map { it.uid })
        assertEquals(listOf("UID SEARCH UID 1:12 UNSEEN"), server.unseenSearches())
        assertEquals(2, res.olderUnseen)
    }

    @Test
    fun `with ESEARCH the server sends only the count`() = runBlocking {
        val server = inbox("IMAP4rev1 ESEARCH")
        val res = session(server).syncFolder("INBOX", 2, emptyMap(), null, olderUnseen = true)
        assertEquals(listOf("UID SEARCH RETURN (COUNT) UID 1:12 UNSEEN"), server.unseenSearches())
        assertEquals(2, res.olderUnseen)
    }

    @Test
    fun `a window that holds the whole folder leaves nothing older to search`() = runBlocking {
        val server = inbox()
        val res = session(server).syncFolder("INBOX", 10, emptyMap(), null, olderUnseen = true)
        assertEquals(5, res.messages.size)
        assertEquals(0, res.olderUnseen)
        assertEquals(emptyList<String>(), server.unseenSearches())
    }

    @Test
    fun `a server that turns the search down gives no count, and the sync still succeeds`() = runBlocking {
        val server = inbox().apply { refuseSearch = true }
        val s = session(server)
        val res = s.syncFolder("INBOX", 2, emptyMap(), null, olderUnseen = true)
        assertEquals(listOf(13L, 14L), res.messages.map { it.uid })
        assertNull(res.olderUnseen)
        // The connection is still good for the next sync.
        server.refuseSearch = false
        assertEquals(2, s.syncFolder("INBOX", 2, res.messages.associateBy { it.uid }, res.uidValidity, olderUnseen = true).olderUnseen)
    }

    @Test
    fun `a sync that doesn't ask doesn't search`() = runBlocking {
        val server = inbox()
        val res = session(server).syncFolder("INBOX", 2, emptyMap(), null)
        assertNull(res.olderUnseen)
        assertFalse(server.commands.any { it.startsWith("UID SEARCH") })
    }

    @Test
    fun `search answers become a count, from ESEARCH or from the uids`() {
        fun count(vararg lines: String) = ImapSession.searchCount(lines.map { IMAPResponse(it) as Response }.toTypedArray())
        assertEquals(434, count("* ESEARCH (TAG \"A7\") UID COUNT 434"))
        // An ESEARCH answer without a COUNT found nothing.
        assertEquals(0, count("* ESEARCH (TAG \"A7\") UID"))
        assertEquals(3, count("* SEARCH 3 7 4294967295"))
        assertEquals(0, count("* SEARCH"))
    }
}
