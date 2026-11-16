package nl.bvdm.rukoo.ui.root

import kotlinx.serialization.Serializable
import nl.bvdm.rukoo.mail.model.MailView
import nl.bvdm.rukoo.mail.model.QuickFilter as ListFilter

/** The panes the layouts arrange (spec 6 §5.2-5.3). */
@Serializable
enum class Pane { LIST, READER, CHAT, COMPOSER }

/** Full-screen surfaces over the panes, in both layouts. */
@Serializable
enum class Overlay { SETTINGS, ONBOARDING }

/** What fills the window: one pane, an overlay, or (in two panes) a left and right pane. */
data class PaneLayout(val left: Pane, val right: Pane)

/**
 * Navigation for both layouts in one state, so folding and unfolding keep the open email, chat and
 * composer (spec 6 §5.6). The pane flags say what is open; [stack] is the one-pane back stack, bottom
 * first, always starting with LIST. Every change is a pure function below, so it is unit-tested.
 */
@Serializable
data class NavState(
    val view: MailView = MailView.AllInboxes,
    /** The selected email. It stays selected after Back in one pane, so unfolding shows it next to the list. */
    val openMessageId: String? = null,
    val chatOpen: Boolean = false,
    /** Null while a new chat has no message yet; set by [chatCreated]. */
    val chatConversationId: String? = null,
    /** The email the chat is about, if any. */
    val chatEmailId: String? = null,
    /** In two panes with the chat open: what sits next to it, the list or the reader. */
    val chatLeft: Pane = Pane.LIST,
    val composerOpen: Boolean = false,
    val stack: List<Pane> = listOf(Pane.LIST),
    /** Updated on tap, scroll and focus; decides which pane stays when the phone folds. */
    val lastInteracted: Pane = Pane.LIST,
    val overlay: Overlay? = null,
    val drawerOpen: Boolean = false,
    val chatListOpen: Boolean = false,
    /** The list's quick filter (All, Unread, Starred, Attachments). */
    val filter: ListFilter = ListFilter.ALL,
    /** The search field is open in the list's top bar. */
    val searching: Boolean = false,
    val query: String = "",
    /** Search every folder of every account (the engine's "everything" view), or only the current view. */
    val searchAll: Boolean = true,
    /** Checked rows: selection mode while not empty. */
    val checked: List<String> = emptyList(),
    /** Stacks the user opened, by their key (day label and sender). */
    val openStacks: List<String> = emptyList(),
    /** The inbox category tab on screen, or null for All. */
    val category: String? = null,
    /**
     * The email last opened while it was unread (desktop's keptId): the Unread filter keeps listing it, read now,
     * until another email opens, also after Back on one pane. Always the open email or null.
     */
    val keptId: String? = null,
) {
    val top: Pane get() = stack.last()

    /** The query the list runs: empty unless the search field is open. */
    val activeQuery: String get() = if (searching) query.trim() else ""

    /** The pane pair on a wide window (spec 6 §5.3). */
    fun twoPanes(): PaneLayout = when {
        chatOpen && composerOpen -> PaneLayout(Pane.COMPOSER, Pane.CHAT)
        chatOpen -> PaneLayout(if (chatLeft == Pane.READER && openMessageId != null) Pane.READER else Pane.LIST, Pane.CHAT)
        composerOpen -> PaneLayout(Pane.LIST, Pane.COMPOSER)
        else -> PaneLayout(Pane.LIST, Pane.READER)
    }

    private fun push(pane: Pane): NavState = if (pane == Pane.LIST) copy(stack = listOf(Pane.LIST)) else copy(stack = stack.filterNot { it == pane } + pane)

    private fun drop(pane: Pane): NavState = copy(stack = stack.filterNot { it == pane && it != Pane.LIST }.ifEmpty { listOf(Pane.LIST) })

    /**
     * A view or folder from the drawer. One pane goes to the list; a chat or composer stays open for two
     * panes. Like desktop's changeView, the filter, search and checks start over, and the open email closes
     * (it would look like it belongs to the new folder) unless the chat beside it is about it.
     */
    fun selectView(next: MailView): NavState {
        val keep = chatOpen && chatEmailId != null && chatEmailId == openMessageId
        return copy(
            view = next,
            drawerOpen = false,
            lastInteracted = Pane.LIST,
            filter = ListFilter.ALL,
            searching = false,
            query = "",
            checked = emptyList(),
            openMessageId = if (keep) openMessageId else null,
            chatLeft = if (keep) chatLeft else Pane.LIST,
            keptId = null,
        ).push(Pane.LIST)
    }

    fun setFilter(next: ListFilter): NavState = copy(filter = next, lastInteracted = Pane.LIST)

    fun openSearch(): NavState = copy(searching = true, lastInteracted = Pane.LIST).push(Pane.LIST)

    fun setQuery(text: String): NavState = copy(query = text)

    fun setSearchAll(all: Boolean): NavState = copy(searchAll = all)

    fun closeSearch(): NavState = copy(searching = false, query = "", checked = emptyList())

    /** Long-press or a tap in selection mode: checks or unchecks one row. */
    fun toggleChecked(id: String): NavState = copy(checked = if (id in checked) checked - id else checked + id, lastInteracted = Pane.LIST)

    fun setChecked(ids: List<String>): NavState = copy(checked = ids.distinct())

    fun clearChecked(): NavState = if (checked.isEmpty()) this else copy(checked = emptyList())

    /** Another category tab. A selection doesn't carry over: its emails aren't shown there to act on. */
    fun setCategory(id: String?): NavState = copy(category = id, checked = emptyList(), lastInteracted = Pane.LIST)

    fun toggleStack(key: String): NavState = copy(openStacks = if (key in openStacks) openStacks - key else openStacks + key)

    /** Opens [id] in the reader. The email the Unread filter kept leaves it, unless it is [id] again. */
    fun openMessage(id: String): NavState =
        copy(openMessageId = id, keptId = keptId.takeIf { it == id }, chatLeft = Pane.READER, lastInteracted = Pane.READER, chatListOpen = false)
            .push(Pane.READER)

    /** The open email [id] was unread when it opened: the Unread filter keeps it once it is read ([keptId]). */
    fun keep(id: String): NavState = if (openMessageId != id || keptId == id) this else copy(keptId = id)

    /** Clears the selection (the email was deleted or moved away). */
    fun messageGone(id: String): NavState {
        val s = if (keptId == id) copy(keptId = null) else this
        return if (s.openMessageId != id) s else s.copy(openMessageId = null, chatLeft = Pane.LIST).drop(Pane.READER)
    }

    fun openChat(conversationId: String?, emailId: String?): NavState = copy(
        chatOpen = true,
        chatConversationId = conversationId,
        chatEmailId = emailId,
        // A chat about an email sits next to that email.
        openMessageId = emailId ?: openMessageId,
        keptId = keptId.takeIf { it == (emailId ?: openMessageId) },
        chatLeft = if (emailId != null) Pane.READER else Pane.LIST,
        chatListOpen = false,
        lastInteracted = Pane.CHAT,
    ).push(Pane.CHAT)

    /**
     * The open chat switches to [conversationId] about [emailId] (the reader beside it moved on) without
     * coming forward: the stack and the pane used last stay as they are.
     */
    fun followEmail(conversationId: String?, emailId: String?): NavState =
        if (!chatOpen) this else copy(chatConversationId = conversationId, chatEmailId = emailId)

    fun chatCreated(conversationId: String): NavState = if (chatOpen) copy(chatConversationId = conversationId) else this

    fun closeChat(): NavState =
        copy(chatOpen = false, chatConversationId = null, chatEmailId = null, chatLeft = Pane.LIST, lastInteracted = Pane.LIST).drop(Pane.CHAT)

    /** Two panes with the chat: bring the list back next to it. One pane: back to the list. */
    fun showList(twoPane: Boolean): NavState =
        if (twoPane) copy(chatLeft = Pane.LIST, lastInteracted = Pane.LIST) else copy(lastInteracted = Pane.LIST).push(Pane.LIST)

    /**
     * The composer opened. [byUser]: from a button, so it comes to the front. An agent's draft (write_draft)
     * doesn't push it over the chat on one pane: the chat's draft card opens it ([showComposer]).
     */
    fun composerOpened(byUser: Boolean): NavState {
        val opened = copy(composerOpen = true)
        return if (byUser || top != Pane.CHAT) opened.copy(lastInteracted = Pane.COMPOSER).push(Pane.COMPOSER) else opened
    }

    fun showComposer(): NavState = if (composerOpen) copy(lastInteracted = Pane.COMPOSER).push(Pane.COMPOSER) else this

    fun composerClosed(): NavState = copy(composerOpen = false, lastInteracted = if (lastInteracted == Pane.COMPOSER) Pane.LIST else lastInteracted).drop(Pane.COMPOSER)

    fun interacted(pane: Pane): NavState = if (lastInteracted == pane) this else copy(lastInteracted = pane)

    fun openOverlay(overlay: Overlay): NavState = copy(overlay = overlay, drawerOpen = false, chatListOpen = false)

    fun closeOverlay(): NavState = copy(overlay = null)

    /**
     * One step back, or null when there is nothing left to close (the system then leaves the app).
     * Sheets and overlays close first. Two panes close the chat column, then the composer. One pane pops
     * its stack: a chat closes, the composer closes, the reader goes back to the list but stays selected.
     */
    fun back(twoPane: Boolean): NavState? {
        if (chatListOpen) return copy(chatListOpen = false)
        if (drawerOpen) return copy(drawerOpen = false)
        if (overlay != null) return closeOverlay()
        // Selection mode and the search field belong to the list: they close before the list goes anywhere.
        val listShown = twoPane || top == Pane.LIST
        if (listShown && checked.isNotEmpty()) return clearChecked()
        if (twoPane) {
            if (chatOpen) return closeChat()
            if (composerOpen) return composerClosed()
            if (searching) return closeSearch()
            return null
        }
        return when (top) {
            Pane.LIST -> if (searching) closeSearch() else null
            Pane.READER -> copy(lastInteracted = Pane.LIST).drop(Pane.READER)
            Pane.CHAT -> closeChat()
            Pane.COMPOSER -> composerClosed()
        }
    }

    /**
     * Two panes became one (spec 6 §5.6): the pane the user worked in last comes to the front. A
     * composer with unsaved text wins, and so does a focused chat input; list + reader with an email
     * open shows the reader.
     */
    fun folded(composerDirty: Boolean, chatFocused: Boolean): NavState {
        val listOrReader = if (openMessageId != null && twoPanes() == PaneLayout(Pane.LIST, Pane.READER)) Pane.READER else Pane.LIST
        val front = when {
            composerOpen && composerDirty -> Pane.COMPOSER
            chatOpen && chatFocused -> Pane.CHAT
            lastInteracted == Pane.CHAT && chatOpen -> Pane.CHAT
            lastInteracted == Pane.COMPOSER && composerOpen -> Pane.COMPOSER
            lastInteracted == Pane.READER && openMessageId != null -> Pane.READER
            else -> listOrReader
        }
        val canonical = buildList {
            add(Pane.LIST)
            if (openMessageId != null) add(Pane.READER)
            if (composerOpen) add(Pane.COMPOSER)
            if (chatOpen) add(Pane.CHAT)
        }
        val ordered = if (front == Pane.LIST) listOf(Pane.LIST) else canonical.filterNot { it == front } + front
        return copy(stack = ordered)
    }
}
