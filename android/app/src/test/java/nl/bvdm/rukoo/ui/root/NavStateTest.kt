package nl.bvdm.rukoo.ui.root

import nl.bvdm.rukoo.core.RukooJson
import nl.bvdm.rukoo.mail.model.MailView
import nl.bvdm.rukoo.mail.model.ViewKind
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class NavStateTest {
    private val start = NavState()

    @Test
    fun onePaneGoesListReaderChatAndBack() {
        val chat = start.openMessage("m1").openChat(null, "m1")
        assertEquals(listOf(Pane.LIST, Pane.READER, Pane.CHAT), chat.stack)
        val reader = chat.back(twoPane = false)!!
        assertFalse(reader.chatOpen)
        assertEquals(Pane.READER, reader.top)
        val list = reader.back(twoPane = false)!!
        assertEquals(Pane.LIST, list.top)
        // Still selected, so unfolding shows it next to the list.
        assertEquals("m1", list.openMessageId)
        assertNull(list.back(twoPane = false))
    }

    @Test
    fun twoPanesFollowTheTable() {
        assertEquals(PaneLayout(Pane.LIST, Pane.READER), start.twoPanes())
        val withEmail = start.openMessage("m1").openChat(null, "m1")
        assertEquals(PaneLayout(Pane.READER, Pane.CHAT), withEmail.twoPanes())
        assertEquals(PaneLayout(Pane.LIST, Pane.CHAT), withEmail.showList(twoPane = true).twoPanes())
        val noEmail = start.openChat("c1", null)
        assertEquals(PaneLayout(Pane.LIST, Pane.CHAT), noEmail.twoPanes())
        // A source opened from the chat replaces the list next to it.
        assertEquals(PaneLayout(Pane.READER, Pane.CHAT), noEmail.openMessage("m2").twoPanes())
        assertEquals(PaneLayout(Pane.LIST, Pane.COMPOSER), start.composerOpened(byUser = true).twoPanes())
        assertEquals(PaneLayout(Pane.COMPOSER, Pane.CHAT), withEmail.composerOpened(byUser = false).twoPanes())
    }

    @Test
    fun twoPanesCloseTheChatFirst() {
        val s = start.openMessage("m1").composerOpened(byUser = true).openChat(null, "m1")
        val noChat = s.back(twoPane = true)!!
        assertFalse(noChat.chatOpen)
        assertTrue(noChat.composerOpen)
        val noComposer = noChat.back(twoPane = true)!!
        assertFalse(noComposer.composerOpen)
        assertNull(noComposer.back(twoPane = true))
    }

    @Test
    fun sheetsAndOverlaysCloseBeforePanes() {
        val s = start.openMessage("m1").copy(chatListOpen = true)
        assertFalse(s.back(false)!!.chatListOpen)
        val settings = start.openMessage("m1").openOverlay(Overlay.SETTINGS)
        val closed = settings.back(twoPane = true)!!
        assertNull(closed.overlay)
        assertEquals("m1", closed.openMessageId)
    }

    @Test
    fun anAgentsDraftDoesNotCoverTheChatOnOnePane() {
        val chat = start.openChat("c1", null)
        val drafted = chat.composerOpened(byUser = false)
        assertTrue(drafted.composerOpen)
        assertEquals(Pane.CHAT, drafted.top)
        assertEquals(Pane.COMPOSER, drafted.showComposer().top)
        // From the list it does come to the front.
        assertEquals(Pane.COMPOSER, start.composerOpened(byUser = false).top)
    }

    @Test
    fun foldingShowsThePaneTouchedLast() {
        val twoPanes = start.openMessage("m1").openChat(null, "m1")
        assertEquals(Pane.CHAT, twoPanes.folded(composerDirty = false, chatFocused = false).top)
        val readerTouched = twoPanes.interacted(Pane.READER).folded(false, false)
        assertEquals(Pane.READER, readerTouched.top)
        // Back from there reaches the chat, then the list.
        assertEquals(Pane.CHAT, readerTouched.back(false)!!.top)
        // List + reader with an email open: the reader.
        assertEquals(Pane.READER, start.openMessage("m1").interacted(Pane.LIST).folded(false, false).top)
        assertEquals(Pane.LIST, start.folded(false, false).top)
        // Unsaved text in the composer wins.
        val composing = twoPanes.composerOpened(byUser = false).interacted(Pane.CHAT)
        assertEquals(Pane.COMPOSER, composing.folded(composerDirty = true, chatFocused = false).top)
        assertEquals(Pane.CHAT, composing.interacted(Pane.LIST).folded(composerDirty = false, chatFocused = true).top)
    }

    @Test
    fun choosingAFolderReturnsToTheList() {
        val s = start.openMessage("m1").copy(drawerOpen = true).selectView(MailView(ViewKind.SENT))
        assertEquals(listOf(Pane.LIST), s.stack)
        assertFalse(s.drawerOpen)
        assertEquals(ViewKind.SENT, s.view.kind)
    }

    @Test
    fun aGoneMessageIsDeselected() {
        val s = start.openMessage("m1").messageGone("m1")
        assertNull(s.openMessageId)
        assertEquals(Pane.LIST, s.top)
        assertEquals("m2", start.openMessage("m2").messageGone("m1").openMessageId)
    }

    @Test
    fun survivesSavedState() {
        val s = start.openMessage("demo:INBOX:12").openChat("c_1", "demo:INBOX:12").selectView(MailView.folder("demo", "Travel"))
        val json = RukooJson.encodeToString(NavState.serializer(), s)
        assertEquals(s, RukooJson.decodeFromString(NavState.serializer(), json))
    }

    @Test
    fun choosingAViewStartsTheListOver() {
        val s = start.openMessage("m1").setFilter(nl.bvdm.rukoo.mail.model.QuickFilter.UNREAD).openSearch().setQuery("call").toggleChecked("m2")
        val next = s.selectView(MailView(ViewKind.STARRED))
        assertEquals(nl.bvdm.rukoo.mail.model.QuickFilter.ALL, next.filter)
        assertFalse(next.searching)
        assertEquals("", next.query)
        assertTrue(next.checked.isEmpty())
        // The open email would look like it belongs to Starred.
        assertNull(next.openMessageId)
        // Unless the chat beside it is about it.
        val chat = start.openMessage("m1").openChat(null, "m1").selectView(MailView(ViewKind.SENT))
        assertEquals("m1", chat.openMessageId)
    }

    @Test
    fun backClearsTheSelectionThenTheSearch() {
        val s = start.openSearch().setQuery("x").toggleChecked("m1").toggleChecked("m2")
        assertEquals(listOf("m1", "m2"), s.checked)
        val unchecked = s.back(twoPane = false)!!
        assertTrue(unchecked.checked.isEmpty())
        assertTrue(unchecked.searching)
        val closed = unchecked.back(twoPane = false)!!
        assertFalse(closed.searching)
        assertEquals("", closed.activeQuery)
        assertNull(closed.back(twoPane = false))
        // In two panes the chat closes before the search does.
        val two = start.openSearch().setQuery("x").openChat("c1", null)
        assertFalse(two.back(twoPane = true)!!.chatOpen)
        assertTrue(two.back(twoPane = true)!!.searching)
    }

    @Test
    fun checksAndStacksToggle() {
        val s = start.toggleChecked("a").toggleChecked("b").toggleChecked("a")
        assertEquals(listOf("b"), s.checked)
        assertEquals(listOf("k"), start.toggleStack("k").openStacks)
        assertTrue(start.toggleStack("k").toggleStack("k").openStacks.isEmpty())
        assertEquals("", start.setQuery("call").activeQuery)
        assertEquals("call", start.openSearch().setQuery(" call ").activeQuery)
    }

    @Test
    fun `the Unread filter keeps the email opened while unread until another opens`() {
        val unread = start.setFilter(nl.bvdm.rukoo.mail.model.QuickFilter.UNREAD)
        val kept = unread.openMessage("m1").keep("m1")
        assertEquals("m1", kept.keptId)
        // Back on one pane, another tab or filter, and opening it again keep it.
        assertEquals("m1", kept.back(twoPane = false)!!.keptId)
        assertEquals("m1", kept.setCategory("money").keptId)
        assertEquals("m1", kept.setFilter(nl.bvdm.rukoo.mail.model.QuickFilter.ALL).keptId)
        assertEquals("m1", kept.openMessage("m1").keptId)
        // Another email lets it go, and so does a chat about another email.
        assertNull(kept.openMessage("m2").keptId)
        assertNull(kept.openChat(null, "m2").keptId)
        assertEquals("m1", kept.openChat(null, null).keptId)
        // Only the open email is kept: a summary that arrives after another opened doesn't count.
        assertNull(unread.openMessage("m1").openMessage("m2").keep("m1").keptId)
        // Another view starts the list over; a deleted or moved email is gone.
        assertNull(kept.selectView(MailView(ViewKind.SENT)).keptId)
        assertNull(kept.messageGone("m1").keptId)
        assertNull(kept.back(twoPane = false)!!.messageGone("m1").keptId)
        assertEquals("m1", kept.messageGone("m2").keptId)
        // It comes back with the rest of the state.
        assertEquals(kept, RukooJson.decodeFromString(NavState.serializer(), RukooJson.encodeToString(NavState.serializer(), kept)))
    }

    @Test
    fun `another category tab clears the selection, and the tab survives a restart`() {
        val picked = NavState().setChecked(listOf("a", "b"))
        val tabbed = picked.setCategory("money")
        assertEquals("money", tabbed.category)
        assertTrue("a selection doesn't carry over to another tab", tabbed.checked.isEmpty())
        assertNull(tabbed.setCategory(null).category)
        assertEquals(tabbed, RukooJson.decodeFromString(NavState.serializer(), RukooJson.encodeToString(NavState.serializer(), tabbed)))
    }
}
