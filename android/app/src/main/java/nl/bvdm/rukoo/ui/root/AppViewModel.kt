package nl.bvdm.rukoo.ui.root

import android.content.Context
import android.net.Uri
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.lifecycle.SavedStateHandle
import androidx.lifecycle.ViewModel
import androidx.lifecycle.ViewModelProvider
import androidx.lifecycle.createSavedStateHandle
import androidx.lifecycle.viewModelScope
import androidx.lifecycle.viewmodel.initializer
import androidx.lifecycle.viewmodel.viewModelFactory
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeoutOrNull
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.serialization.builtins.ListSerializer
import kotlinx.serialization.builtins.MapSerializer
import kotlinx.serialization.builtins.serializer
import nl.bvdm.rukoo.AppServices
import nl.bvdm.rukoo.NotificationRoute
import nl.bvdm.rukoo.R
import nl.bvdm.rukoo.core.RukooJson
import nl.bvdm.rukoo.mail.MailException
import nl.bvdm.rukoo.mail.compose.toOutgoing
import nl.bvdm.rukoo.mail.model.Address
import nl.bvdm.rukoo.mail.model.FolderRole
import nl.bvdm.rukoo.mail.model.MailView
import nl.bvdm.rukoo.mail.model.OutgoingAttachment
import nl.bvdm.rukoo.mail.model.ReplyKind
import nl.bvdm.rukoo.mail.model.ViewKind
import nl.bvdm.rukoo.mail.model.QuickFilter as ListFilter
import nl.bvdm.rukoo.ui.ComposerState
import nl.bvdm.rukoo.ui.agent.DraftCards
import nl.bvdm.rukoo.ui.PendingRebind
import nl.bvdm.rukoo.ui.MailtoFields
import nl.bvdm.rukoo.ui.composer.ComposerFiles
import nl.bvdm.rukoo.ui.composer.Recipients
import nl.bvdm.rukoo.ui.mail.DialogChoice
import nl.bvdm.rukoo.ui.mail.DialogRequest
import nl.bvdm.rukoo.ui.mail.ListModel
import nl.bvdm.rukoo.ui.mail.MailUi
import nl.bvdm.rukoo.ui.mail.MailUiHost

/** Things the root does with a Context: open a link in a Custom Tab. */
sealed interface UiEffect {
    data class OpenUrl(val url: String) : UiEffect
}

/**
 * Navigation state for both layouts, above the layout so folding keeps everything (spec 6 §5.6), and
 * the way panes reach the app's services ([services]). Intents are plain calls; the state machine is
 * [NavState]. Saved in the SavedStateHandle so a recreated activity comes back where it was. Mail
 * actions ([mailUi]) and the composer's actions run in the view model's scope, so they finish when the
 * pane that started them leaves the screen.
 */
class AppViewModel(
    val services: AppServices,
    private val saved: SavedStateHandle = SavedStateHandle(),
) : ViewModel(), MailUiHost {
    private val _state = MutableStateFlow(restore())
    val state: StateFlow<NavState> = _state.asStateFlow()

    private val twoPaneFlow = MutableStateFlow(false)

    /** Whether the window shows two panes; RukooRoot reports it from the window width. */
    val twoPane: Boolean get() = twoPaneFlow.value

    /** Set by the chat input on focus, so folding keeps the chat in front. */
    var chatFocused: Boolean = false

    val mailUi = MailUi(services.mail, viewModelScope, this, categories = services.categories)

    private val _effects = MutableSharedFlow<UiEffect>(extraBufferCapacity = 8)
    val effects: SharedFlow<UiEffect> = _effects

    /** True while Send runs. */
    var sending by mutableStateOf(false)
        private set

    // The rows the list shows, in order, as the list last reported them: Previous and Next walk them, and a
    // moved message hands the reader to the next one. Observable, so the reader's buttons follow new mail.
    private var shownIds: List<String> by mutableStateOf(emptyList())

    private val _chatEmails = MutableStateFlow<List<String>>(emptyList())

    /**
     * Emails "Add to chat" asked for that the chat input hasn't taken yet (track D's ChatPane takes them with
     * [takeChatEmails]); the first is also the chat's email when a new chat opens.
     */
    val chatEmails: StateFlow<List<String>> = _chatEmails.asStateFlow()

    init {
        // The composer can open without the UI: an agent's write_draft calls ComposerController.open.
        viewModelScope.launch {
            services.composer.state.map { it != null }.distinctUntilChanged().collect { open ->
                set { s ->
                    when {
                        open && !s.composerOpen -> s.composerOpened(byUser = false)
                        !open && s.composerOpen -> s.composerClosed()
                        else -> s
                    }
                }
            }
        }
        // What the tools see (get_context, rukoo/hello).
        viewModelScope.launch {
            combine(state, services.composer.state, twoPaneFlow) { nav, composer, two -> Triple(nav, composer, two) }.collect { (nav, composer, two) ->
                services.screen.update {
                    it.copy(
                        view = nav.view,
                        openMessageId = nav.openMessageId,
                        selectedIds = nav.checked,
                        composer = composer,
                        chatConversationId = if (chatVisible(nav, two)) nav.chatConversationId else null,
                    )
                }
            }
        }
        // The chat on screen gets no unread mark when its turn ends.
        viewModelScope.launch {
            combine(state, twoPaneFlow, services.screen.state.map { it.foreground }.distinctUntilChanged()) { nav, two, foreground ->
                if (foreground && chatVisible(nav, two)) nav.chatConversationId else null
            }.distinctUntilChanged().collect { services.agents.watch(it) }
        }
    }

    private fun chatVisible(nav: NavState, two: Boolean): Boolean =
        nav.chatOpen && nav.overlay == null && (if (two) nav.twoPanes().right == Pane.CHAT else nav.top == Pane.CHAT)

    /** Whether the chat is on screen right now (the list's news dot leaves it out). */
    fun chatOnScreen(nav: NavState = _state.value): Boolean = chatVisible(nav, twoPane)

    private fun restore(): NavState =
        saved.get<String>(KEY)?.let { runCatching { RukooJson.decodeFromString(NavState.serializer(), it) }.getOrNull() } ?: NavState()

    private fun set(transform: (NavState) -> NavState) {
        val before = _state.value
        val after = transform(before)
        if (after == before) return
        _state.value = after
        saved[KEY] = RukooJson.encodeToString(NavState.serializer(), after)
    }

    /** Called by RukooRoot whenever the window crosses 600dp. */
    fun onLayout(twoPane: Boolean) {
        val was = twoPaneFlow.value
        twoPaneFlow.value = twoPane
        if (was && !twoPane) set { it.folded(services.composer.current()?.dirty == true, chatFocused) }
    }

    fun interacted(pane: Pane) = set { it.interacted(pane) }

    fun selectView(view: MailView) {
        set { it.selectView(view) }
        val account = view.accountId
        val folder = view.folder
        if (account != null && folder != null) viewModelScope.launch { runCatching { services.mail.openFolder(account, folder) } }
    }

    /**
     * Opens [id] in the reader and marks it read, as desktop's openMessage does (drafts stay as they are). An email
     * that was unread is kept first, so the Unread filter still lists it once the read flag lands.
     */
    fun openMessage(id: String) {
        set { it.openMessage(id) }
        viewModelScope.launch {
            val m = runCatching { services.mail.summary(id) }.getOrNull() ?: return@launch
            if (!m.unread || m.role == FolderRole.DRAFTS) return@launch
            set { it.keep(id) }
            runCatching { services.mail.setFlags(listOf(id), unread = false) }
        }
    }

    fun messageGone(id: String) = set { it.messageGone(id) }

    /** The list tells which rows it shows, in order. */
    fun listShown(ids: List<String>) {
        shownIds = ids
    }

    // Where each list (view, folder, search) was scrolled to, so going back to a folder finds the same place
    // (spec 6 §5.6), also after the process was gone: they are kept in the SavedStateHandle. The list on
    // screen keeps its own place (rememberSaveable). Oldest first, so the oldest go once there are too many.
    private val listPositions: LinkedHashMap<String, List<Int>> = LinkedHashMap(
        saved.get<String>(POSITIONS)?.let { runCatching { RukooJson.decodeFromString(POSITIONS_SERIALIZER, it) }.getOrNull() }.orEmpty(),
    )

    fun listPosition(key: String): Pair<Int, Int>? = listPositions[key]?.takeIf { it.size == 2 }?.let { it[0] to it[1] }

    fun keepListPosition(key: String, index: Int, offset: Int) {
        listPositions.remove(key)
        listPositions[key] = listOf(index, offset)
        while (listPositions.size > MAX_POSITIONS) listPositions.remove(listPositions.keys.first())
        saved[POSITIONS] = RukooJson.encodeToString(POSITIONS_SERIALIZER, listPositions)
    }

    /** The email above or below the open one in the list; null at either end. */
    fun neighbour(id: String, step: Int): String? {
        val i = shownIds.indexOf(id)
        if (i < 0) return null
        return shownIds.getOrNull(i + step)
    }

    // List state

    fun setFilter(filter: ListFilter) = set { it.setFilter(filter) }

    fun openSearch() = set { it.openSearch() }

    fun setQuery(text: String) = set { it.setQuery(text) }

    fun setSearchAll(all: Boolean) = set { it.setSearchAll(all) }

    fun closeSearch() = set { it.closeSearch() }

    fun toggleChecked(id: String) = set { it.toggleChecked(id) }

    fun setChecked(ids: List<String>) = set { it.setChecked(ids) }

    fun clearChecked() = set { it.clearChecked() }

    fun toggleStack(key: String) = set { it.toggleStack(key) }

    fun setCategory(id: String?) = set { it.setCategory(id) }

    fun setSort(sort: nl.bvdm.rukoo.mail.model.SortOrder) {
        viewModelScope.launch { services.settings.update { it.copy(sort = sort) } }
    }

    fun setDensity(density: nl.bvdm.rukoo.settings.Density) {
        viewModelScope.launch { services.settings.update { it.copy(density = density) } }
    }

    // MailUiHost

    override fun detach(ids: Collection<String>): Boolean {
        val nav = _state.value
        val open = nav.openMessageId
        val wasOpen = open != null && open in ids
        set { it.copy(checked = it.checked - ids.toSet()) }
        if (wasOpen) {
            val next = ListModel.nextAfter(shownIds, open, ids)
            // The reader goes on to the next email like desktop's detach; from the list, the email just leaves.
            if (next != null && (twoPane || nav.top == Pane.READER)) openMessage(next) else messageGone(open!!)
        }
        shownIds = shownIds - ids.toSet()
        return wasOpen
    }

    override fun reopen(id: String) = openMessage(id)

    override fun openUrl(url: String) {
        _effects.tryEmit(UiEffect.OpenUrl(url))
    }

    override fun composeMailto(fields: MailtoFields) = newMessage(fields)

    override fun showFolder(accountId: String, path: String) = selectView(MailView.folder(accountId, path))

    // Chat

    /**
     * Opens a chat: an existing one by id, or a new one about [emailId]. Without an [emailId] the chat's own
     * email is used, and a new chat is about the open email, unless [withoutEmail]: then it is about none.
     */
    fun openChat(conversationId: String? = null, emailId: String? = null, withoutEmail: Boolean = false) {
        val email = when {
            withoutEmail -> null
            emailId != null -> emailId
            conversationId != null -> services.agents.get(conversationId)?.message?.id
            else -> _state.value.openMessageId
        }
        set { it.openChat(conversationId, email) }
    }

    /**
     * The open chat follows another email (two panes: the reader beside it moved on). Unlike [openChat] it
     * doesn't bring the chat forward or count as using it, so folding afterwards still shows the reader.
     */
    fun followEmail(conversationId: String?, emailId: String?) = set { it.followEmail(conversationId, emailId) }

    /**
     * A notification tap. The inbox of an account needs the accounts loaded, which on a cold start they
     * aren't yet: it waits for them, briefly.
     */
    fun openRoute(route: NotificationRoute) {
        when (route) {
            is NotificationRoute.Composer -> if (services.composer.isOpen) showComposer() else route.chat?.let(::openRoute)
            is NotificationRoute.Chat -> openChat(route.conversationId, route.messageId)
            is NotificationRoute.Message -> {
                openMessage(route.messageId)
                showCategoryOf(route.messageId)
            }
            is NotificationRoute.Inbox -> viewModelScope.launch {
                withTimeoutOrNull(LOAD_WAIT_MS) { services.mail.loaded.first { it } }
                val accounts = services.mail.accounts.value
                // With one account its inbox is All inboxes already.
                val one = route.accountId.takeIf { id -> accounts.size > 1 && accounts.any { it.id == id } }
                selectView(MailView(ViewKind.INBOX, one))
            }
        }
    }

    /**
     * An email opened from a notification that Inbox or the category on screen leaves out, because Show in Inbox is off
     * for its category or it is in another one, brings up its own category, so its row is on screen beside it (desktop's
     * openMessage). Another account's inbox can't list it whatever the category, so that stays as it is, as on desktop.
     * The pane the user works in stays as the notification left it.
     */
    private fun showCategoryOf(id: String) {
        val categories = services.categories ?: return
        viewModelScope.launch {
            val m = runCatching { services.mail.summary(id) }.getOrNull() ?: return@launch
            fun applies(nav: NavState) = nav.view.kind == ViewKind.INBOX && !nav.searching && nav.openMessageId == id && (nav.view.accountId == null || nav.view.accountId == m.accountId)
            val nav = _state.value
            if (!applies(nav)) return@launch
            val lookup = withContext(Dispatchers.Default) { categories.lookup(services.mail.accounts.value, listOf(m)) }
            val own = lookup[m] ?: return@launch
            val tab = nav.category?.takeIf { tab -> categories.state.value.categories.any { it.id == tab } }
            if (ListModel.categoryRows(listOf(m), lookup, tab, inbox = true).isNotEmpty()) return@launch
            // The user may have moved on while the categories were worked out.
            set { now -> if (applies(now) && now.view == nav.view && now.category == nav.category) now.copy(category = own, checked = emptyList()) else now }
        }
    }

    /**
     * "Add to chat" (desktop addToChatItem): the open chat gets the emails, else a new chat about the first.
     * The chat input takes them from [chatEmails].
     */
    fun addToChat(ids: List<String>) {
        if (ids.isEmpty()) return
        _chatEmails.value = ids.distinct()
        val nav = _state.value
        set { it.clearChecked() }
        // A started chat takes the emails as attachments and stays about its own email; a new one is about the first.
        if (nav.chatOpen && nav.chatConversationId != null) openChat(nav.chatConversationId) else openChat(null, ids.first())
    }

    /** The chat input took the emails [chatEmails] offered. */
    fun takeChatEmails(): List<String> = _chatEmails.value.also { _chatEmails.value = emptyList() }

    /** The first message created the conversation of a new chat. */
    fun chatCreated(conversationId: String) = set { it.chatCreated(conversationId) }

    fun closeChat() = set { it.closeChat() }

    fun showList() = set { it.showList(twoPane) }

    // Composer

    // Saving, deleting and sending the open draft wait for each other, so one finishing can't close or replace
    // a composer another is still working on.
    private val composerWork = Mutex()

    /**
     * Turns the recipient text typed in To, Cc and Bcc ([ComposerState.typed]) into recipients of the open
     * draft, as leaving the field does (desktop flushInputs).
     */
    private fun flushRecipients() {
        services.composer.update { s ->
            if (s.typed.isEmpty()) return@update s
            s.copy(
                to = Recipients.merge(s.to, Recipients.parse(s.typed["to"].orEmpty())),
                cc = Recipients.merge(s.cc, Recipients.parse(s.typed["cc"].orEmpty())),
                bcc = Recipients.merge(s.bcc, Recipients.parse(s.typed["bcc"].orEmpty())),
                typed = emptyMap(),
                dirty = s.dirty || s.typed.values.any { it.isNotBlank() },
            )
        }
    }

    /** Files picked for the draft [composerId], copied in this scope so folding during a large copy loses nothing. */
    fun attachFiles(context: Context, uris: List<Uri>, composerId: String) {
        viewModelScope.launch {
            for (uri in uris) {
                val added = withContext(Dispatchers.IO) { runCatching { ComposerFiles.copyIn(context, uri, composerId) }.getOrNull() }
                if (added == null) {
                    mailUi.toasts.showError(mailUi.strings.get(R.string.composer_attachments_failed, uri.lastPathSegment.orEmpty()))
                    continue
                }
                services.composer.update { if (it.id == composerId) it.copy(attachments = it.attachments + added, dirty = true) else it }
            }
        }
    }

    private fun hasWords(d: ComposerState): Boolean =
        d.body.isNotBlank() || d.subject.isNotBlank() || d.to.isNotEmpty() || d.cc.isNotEmpty() || d.bcc.isNotEmpty() || d.attachments.isNotEmpty()

    /**
     * Whether [d] holds something closing it would lose: recipient text not yet turned into chips, changed
     * words, or an agent's draft that isn't in Drafts as it is now (write_draft may open it clean).
     */
    private fun hasUnsaved(d: ComposerState): Boolean {
        if (d.typed.values.any { it.isNotBlank() }) return true
        if (!hasWords(d)) return false
        if (d.dirty) return true
        return (d.draftedBy != null || d.editedAgent != null) && services.drafts.get(d.id)?.content() != d.content()
    }

    private fun saveFailed() = mailUi.toasts.showError(mailUi.strings.get(R.string.composer_errors_save_local))

    /**
     * Keeps [d]. A draft reopened from the account's Drafts folder ([ComposerState.draftId]) goes back there in
     * place of the stored copy, so desktop sees the change (its draft round-trip); the composer then carries the
     * new copy's id, and its attachments point at the new copy, for the next save. Any other draft is kept on
     * this phone, and so is a server draft the server refused, so nothing is lost. Returns what was kept, as the
     * composer now holds it when nothing changed meanwhile; null when it couldn't be kept at all.
     */
    private suspend fun keep(input: ComposerState): ComposerState? {
        // An earlier save's attachments are pointed at their copy first; until then nothing saves with them.
        val d = settled(input) ?: return null
        val stored = d.draftId
        val accountId = sendAccount(d)
        val from = d.from ?: accountId?.let(::defaultFrom)
        if (stored != null && accountId != null && from != null) {
            try {
                // Null: the server didn't say where the new copy went, and the engine kept the old one.
                val newId = services.mail.saveDraft(d.toOutgoing(accountId, from))
                val pending = newId?.let { PendingRebind(stored, it, d.attachments) }
                val rebound = if (pending == null) emptyMap() else rebound(pending)
                fun ComposerState.moved() = copy(
                    draftId = newId ?: stored,
                    attachments = attachments.map { a -> rebound?.get(a) ?: a },
                    rebind = if (rebound == null) pending else null,
                )
                services.composer.update { if (it.id == d.id) it.moved() else it }
                if (newId != null) storedDraftReplaced(stored, newId)
                // A copy kept on this phone before is out of date now.
                if (services.drafts.contains(d.id)) services.drafts.remove(d.id)
                if (rebound == null) {
                    // Saved, but its attachments couldn't be found on the new copy yet: the composer stays open, and
                    // the next save or Send settles them first. Drafts has the whole draft.
                    attachmentsUnsettled()
                    return null
                }
                return d.moved()
            } catch (e: Exception) {
                mailUi.toasts.showError(mailUi.strings.get(R.string.composer_errors_save_server, e.message ?: e.javaClass.simpleName))
            }
        }
        if (!services.drafts.save(d)) {
            saveFailed()
            return null
        }
        return d
    }

    /**
     * The saved attachments that came from the stored draft [PendingRebind.from], pointed at the copy that
     * replaced it (desktop's remapAttachments): the old copy is deleted, so a second save or Send would find
     * nothing there. The new copy lists them in the order they were saved; the engine keeps its body, so reading
     * it back needs no download. Null when they couldn't be found on it.
     */
    private suspend fun rebound(p: PendingRebind): Map<OutgoingAttachment, OutgoingAttachment>? {
        if (p.saved.none { it.sourceMessageId == p.from }) return emptyMap()
        val copy = runCatching { services.mail.fullMessage(p.to).attachments }.getOrNull() ?: return null
        if (copy.size < p.saved.size) return null
        return p.saved.withIndex()
            .filter { (_, a) -> a.sourceMessageId == p.from }
            .associate { (i, a) -> a to a.copy(sourceMessageId = p.to, sourceIndex = copy[i].index) }
    }

    /**
     * [d] with an earlier save's attachments pointed at their copy ([ComposerState.rebind]), also in the open
     * composer; null, with the error shown, while that copy still can't be read.
     */
    private suspend fun settled(d: ComposerState): ComposerState? {
        val p = d.rebind ?: return d
        val rebound = rebound(p) ?: return null.also { attachmentsUnsettled() }
        fun ComposerState.bound() = copy(attachments = attachments.map { a -> rebound[a] ?: a }, rebind = null)
        services.composer.update { if (it.id == d.id) it.bound() else it }
        return d.bound()
    }

    private fun attachmentsUnsettled() = mailUi.toasts.showError(mailUi.strings.get(R.string.composer_errors_draft_attachments))

    /** The reader showed the stored draft [oldId], which is gone now: it shows its new copy [newId], or nothing. */
    private fun storedDraftReplaced(oldId: String, newId: String?) = set { s ->
        when {
            s.openMessageId != oldId -> s
            newId != null -> s.copy(openMessageId = newId)
            else -> s.messageGone(oldId)
        }
    }

    /**
     * Keeps the open draft [id] in Drafts and closes it, but only once what was kept is what the composer still
     * holds: a word typed, an attachment arriving or an agent writing during the save is saved again rather than
     * thrown away with the composer. False when it couldn't be kept (it stays open) or is no longer open.
     * Runs under [composerWork].
     */
    private suspend fun keepAndClose(id: String): Boolean {
        repeat(SAVE_TRIES) { attempt ->
            if (attempt == 0) flushRecipients()
            val d = services.composer.current()?.takeIf { it.id == id } ?: return false
            val kept = keep(d) ?: return false
            val now = services.composer.current()?.takeIf { it.id == id } ?: return false
            if (now.content() == kept.content()) {
                closeComposerNow()
                return true
            }
        }
        // Still changing after several saves: it stays open, so nothing is lost.
        return false
    }

    /**
     * Opens [draft] in front. An open draft with unsaved words is kept in Drafts first, as desktop saves on
     * leaving; when that save fails, the open draft stays and the new one doesn't open. While a save or Send
     * of the open draft runs, the new one waits for it.
     */
    fun openComposer(draft: ComposerState = ComposerState()) {
        val open = services.composer.current()
        // The same draft open already is the newer copy (edits, or an agent's rewrite): it only comes to the front.
        if (open?.id == draft.id) return showComposer()
        if (!composerWork.isLocked && (open == null || !hasUnsaved(open))) return showNewComposer(draft)
        viewModelScope.launch {
            composerWork.withLock {
                repeat(SAVE_TRIES) { attempt ->
                    val parked = services.composer.current()
                    if (parked?.id == draft.id) return@withLock showComposer()
                    if (parked == null || !hasUnsaved(parked)) return@withLock showNewComposer(draft)
                    if (attempt == 0) flushRecipients()
                    val parking = services.composer.current() ?: return@withLock showNewComposer(draft)
                    val kept = keep(parking) ?: return@withLock
                    val now = services.composer.current()
                    if (now == null || (now.id == kept.id && now.content() == kept.content())) {
                        mailUi.toasts.showToast(mailUi.strings.get(R.string.composer_status_saved_closed))
                        return@withLock showNewComposer(draft)
                    }
                }
            }
        }
    }

    private fun showNewComposer(draft: ComposerState) {
        set { it.composerOpened(byUser = true) }
        services.composer.open(draft)
    }

    private fun account(id: String?) = services.mail.accounts.value.let { list -> list.firstOrNull { it.id == id } ?: list.firstOrNull() }

    private fun defaultAccountId(): String? =
        _state.value.view.accountId ?: services.settingsSnapshot.value.defaultAccountId?.takeIf { id -> services.mail.accounts.value.any { it.id == id } }
            ?: services.mail.accounts.value.firstOrNull()?.id

    /** The From an account sends with by default: its chosen address, else its own. */
    fun defaultFrom(accountId: String): Address? {
        val ids = services.mail.identities(accountId)
        val preferred = account(accountId)?.defaultFrom
        val identity = ids.firstOrNull { preferred != null && it.address.equals(preferred, ignoreCase = true) } ?: ids.firstOrNull { it.primary } ?: ids.firstOrNull()
        return identity?.let { Address(it.name, it.address) }
    }

    /**
     * "New message", or a mailto: link ([fields]): the current account's default From. The account's signature
     * is shown under the text and goes along when it is sent.
     */
    fun newMessage(fields: MailtoFields? = null) {
        // A mailto: link can start the app before the accounts are read; it waits for them, however long that
        // takes, rather than getting lost.
        if (!services.mail.loaded.value) {
            viewModelScope.launch {
                services.mail.loaded.first { it }
                newMessageNow(fields)
            }
            return
        }
        newMessageNow(fields)
    }

    private fun newMessageNow(fields: MailtoFields?) {
        val accountId = defaultAccountId()
        if (accountId == null) {
            mailUi.toasts.showError(mailUi.strings.get(R.string.composer_errors_no_account))
            return
        }
        val base = fields?.toComposer() ?: ComposerState()
        openComposer(base.copy(accountId = accountId, from = defaultFrom(accountId)))
    }

    /**
     * Reply, reply all or forward [messageId], or ([ReplyKind.DRAFT]) edit a draft from the Drafts folder: the
     * engine's fields (MailEngine.replyFields), with desktop's quote. A draft keeps its From, attachments and
     * quote, and replaces the stored copy when it is saved or sent.
     */
    fun reply(messageId: String, kind: ReplyKind) {
        viewModelScope.launch {
            try {
                openComposer(services.mail.replyFields(messageId, kind).toComposerState())
            } catch (e: Exception) {
                mailUi.toasts.showError(e.message ?: e.javaClass.simpleName)
            }
        }
    }

    /** The reader's Edit on a stored draft. */
    fun editDraft(messageId: String) = reply(messageId, ReplyKind.DRAFT)

    /** A draft kept on this phone, opened again. The one that is open already just comes to the front, edits and all. */
    fun openLocalDraft(id: String) {
        if (services.composer.current()?.id == id) return showComposer()
        val draft = services.drafts.get(id) ?: return
        openComposer(draft.copy(dirty = false))
    }

    /** The chat's draft card: bring the composer the agent filled to the front. */
    fun showComposer() = set { it.showComposer() }

    /** Whether closing the composer now would ask "Save draft?" first: whether it holds anything unsaved. */
    fun composerAsks(): Boolean = services.composer.current()?.let(::hasUnsaved) == true

    /**
     * Close (X) or Back on the composer. Unsaved words ask Save, Don't save or Cancel (composer.js:1046-1063);
     * a reopened draft that wasn't changed stays in Drafts. Nothing closes while Send runs.
     */
    fun closeComposer() {
        if (sending) return
        val d = services.composer.current() ?: return set { it.composerClosed() }
        if (!composerAsks()) {
            if (services.drafts.contains(d.id)) mailUi.toasts.showToast(mailUi.strings.get(R.string.composer_status_kept))
            closeComposerNow()
            return
        }
        viewModelScope.launch {
            val s = mailUi.strings
            val answer = mailUi.ask(
                DialogRequest(
                    title = s.get(R.string.composer_close_title),
                    message = s.get(R.string.composer_close_help),
                    confirmLabel = s.get(R.string.common_actions_save),
                    cancelLabel = s.get(R.string.common_actions_cancel),
                    otherLabel = s.get(R.string.composer_close_discard),
                ),
            )
            when (answer.choice) {
                // The composer (and its own copy on disk) closes only once the draft is safely kept.
                DialogChoice.CONFIRM -> composerWork.withLock {
                    if (keepAndClose(d.id)) mailUi.toasts.showToast(s.get(R.string.composer_status_saved_closed))
                }
                DialogChoice.OTHER -> composerWork.withLock {
                    if (services.composer.current()?.id != d.id) return@withLock
                    if (services.drafts.contains(d.id)) {
                        if (!services.drafts.remove(d.id)) return@withLock deleteFailed()
                        mailUi.toasts.showToast(s.get(R.string.composer_status_deleted))
                    }
                    closeComposerNow()
                }
                DialogChoice.CANCEL -> Unit
            }
        }
    }

    private fun deleteFailed() = mailUi.toasts.showError(mailUi.strings.get(R.string.composer_errors_delete_local))

    private fun closeComposerNow() {
        set { it.composerClosed() }
        services.composer.close()
    }

    /**
     * "Delete draft": asks, then throws the draft away, here and in Drafts: on this phone, and the stored copy
     * in the account's Drafts folder when it came from there.
     */
    fun discardDraft() {
        if (sending) return
        val d = services.composer.current() ?: return
        val kept = services.drafts.contains(d.id) || d.draftId != null
        if (!kept && !hasWords(d) && d.typed.values.all { it.isBlank() }) return closeComposerNow()
        viewModelScope.launch {
            val s = mailUi.strings
            val answer = mailUi.ask(
                DialogRequest(
                    title = s.get(R.string.composer_discard_title),
                    message = s.get(if (kept) R.string.composer_discard_saved_help else R.string.composer_discard_unsaved_help),
                    confirmLabel = s.get(R.string.common_actions_delete),
                    cancelLabel = s.get(R.string.common_actions_cancel),
                    danger = true,
                ),
            )
            if (answer.choice != DialogChoice.CONFIRM) return@launch
            composerWork.withLock {
                val current = services.composer.current()?.takeIf { it.id == d.id } ?: return@withLock
                current.draftId?.let { stored ->
                    try {
                        services.mail.discardDraft(stored)
                    } catch (e: MailException) {
                        // Gone from the server already is what the user asked for.
                        if (e.code != "missing") return@withLock mailUi.toasts.showError(e.message ?: s.get(R.string.composer_errors_delete_local))
                    }
                    storedDraftReplaced(stored, null)
                }
                if (!services.drafts.remove(d.id)) return@withLock deleteFailed()
                closeComposerNow()
                if (kept) mailUi.toasts.showToast(s.get(R.string.composer_status_deleted))
            }
        }
    }

    /** "Save to Drafts" from the composer's menu: keeps it and closes, like desktop's Ctrl+S then Close. */
    fun saveDraft() {
        if (sending) return
        val id = services.composer.current()?.id ?: return
        viewModelScope.launch {
            composerWork.withLock {
                if (keepAndClose(id)) mailUi.toasts.showToast(mailUi.strings.get(R.string.composer_status_saved_to_drafts))
            }
        }
    }

    /** What stops [d] from being sent, as a message for the user, or null. */
    private fun sendProblem(d: ComposerState): String? {
        val s = mailUi.strings
        val all = d.to + d.cc + d.bcc
        if (all.isEmpty()) return s.get(R.string.composer_errors_no_recipients)
        all.firstOrNull { !Recipients.isValid(it.address) }?.let { return s.get(R.string.common_errors_invalid_email_value, it.address) }
        val accountId = sendAccount(d) ?: return s.get(R.string.composer_errors_no_account)
        if (d.from == null && defaultFrom(accountId) == null) return s.get(R.string.composer_errors_no_account)
        return null
    }

    private fun sendAccount(d: ComposerState): String? = d.accountId?.takeIf { id -> services.mail.accounts.value.any { it.id == id } } ?: defaultAccountId()

    // The parts of a draft that make up the mail; dirty, the save time and where it is stored don't.
    private fun ComposerState.content() = copy(dirty = false, savedAt = 0, draftedBy = null, editedAgent = null, draftId = null)

    /**
     * Send (composer.js:976-985): typed recipient text joins first, then it needs a valid recipient and asks
     * about an empty subject. While it runs the composer is read-only. If the draft changed anyway (an agent
     * wrote into it), it stays open with the newer text instead of closing.
     */
    fun send() {
        if (sending) return
        flushRecipients()
        val d = services.composer.current() ?: return
        val s = mailUi.strings
        sendProblem(d)?.let { return mailUi.toasts.showError(it) }
        viewModelScope.launch {
            if (d.subject.isBlank()) {
                val answer = mailUi.ask(
                    DialogRequest(
                        s.get(R.string.composer_send_no_subject_title),
                        s.get(R.string.composer_send_no_subject_help),
                        s.get(R.string.composer_actions_send),
                        s.get(R.string.common_actions_cancel),
                    ),
                )
                if (answer.choice != DialogChoice.CONFIRM) return@launch
            }
            // A save of this draft that is still writing finishes first; it can't close the composer under Send.
            composerWork.withLock {
                if (sending) return@withLock
                flushRecipients()
                val open = services.composer.current()?.takeIf { it.id == d.id } ?: return@withLock
                // Attachments an earlier save left pointing at a replaced copy are settled first, or nothing goes.
                // Settling can wait for the server while the composer is still editable: what goes is what it holds
                // afterwards, not the copy from before.
                if (open.rebind != null) {
                    settled(open) ?: return@withLock
                    flushRecipients()
                }
                val current = services.composer.current()?.takeIf { it.id == d.id && it.rebind == null } ?: return@withLock
                sendProblem(current)?.let { return@withLock mailUi.toasts.showError(it) }
                val accountId = sendAccount(current) ?: return@withLock
                val from = current.from ?: defaultFrom(accountId) ?: return@withLock
                sending = true
                mailUi.toasts.showToast(s.get(R.string.composer_status_sending))
                try {
                    try {
                        // The engine builds the mail: the text, the account's signature, the quote (desktop's
                        // markers), and it removes the stored draft this composer edits.
                        services.mail.send(current.toOutgoing(accountId, from))
                    } catch (e: Exception) {
                        return@withLock mailUi.toasts.showError(e.message?.let { s.get(R.string.composer_errors_send_reason, it) } ?: s.get(R.string.composer_errors_send))
                    }
                    // The mail is out; from here nothing may suggest it wasn't. A copy left in Drafts is said so.
                    current.draftId?.let { storedDraftReplaced(it, null) }
                    val cleaned = services.drafts.remove(current.id)
                    val after = services.composer.current()
                    if (after?.id == current.id && after.content() == current.content()) closeComposerNow()
                    if (cleaned) mailUi.toasts.showToast(s.get(R.string.composer_status_sent))
                    else mailUi.toasts.showError(s.get(R.string.composer_status_sent_draft_kept))
                } finally {
                    sending = false
                }
            }
        }
    }

    /** "Drafted by {name} · Undo": asks, then puts back what was there and marks the chat's draft card undone. */
    fun undoAgentDraft() {
        val d = services.composer.current() ?: return
        val name = d.draftedBy ?: d.editedAgent ?: mailUi.strings.get(R.string.composer_agent_someone)
        viewModelScope.launch {
            // As on desktop, only edits the user made after the agent wrote are worth a question.
            if (d.editedAgent != null) {
                val s = mailUi.strings
                val answer = mailUi.ask(
                    DialogRequest(
                        s.get(R.string.composer_agent_undo_title, name),
                        s.get(R.string.composer_agent_undo_help),
                        s.get(R.string.composer_agent_undo),
                        s.get(R.string.common_actions_cancel),
                    ),
                )
                if (answer.choice != DialogChoice.CONFIRM) return@launch
            }
            val since = services.composer.undoAgentDraftSince() ?: return@launch
            // The cards of the writes this Undo took back, in any chat, whatever form their key has.
            runCatching { DraftCards.markUndone(services.agents, d.id, since) }
        }
    }

    fun openSettings() = set { it.openOverlay(Overlay.SETTINGS) }

    // The pages Settings opens on next, bottom first; taken once by the screen.
    private var settingsStart: List<String>? = null

    /** Opens Settings on [pages], the last one showing and Back walking down them. */
    fun openSettings(pages: List<String>) {
        settingsStart = pages
        openSettings()
    }

    fun takeSettingsStart(): List<String>? = settingsStart.also { settingsStart = null }

    fun openOnboarding() = set { it.openOverlay(Overlay.ONBOARDING) }

    fun closeOverlay() = set { it.closeOverlay() }

    fun openDrawer() = set { it.copy(drawerOpen = true) }

    fun closeDrawer() = set { it.copy(drawerOpen = false) }

    fun openChatList() = set { it.copy(chatListOpen = true) }

    fun closeChatList() = set { it.copy(chatListOpen = false) }

    fun canGoBack(): Boolean = _state.value.back(twoPane) != null

    /** Whether Back would stop at the composer's "Save draft?" instead of moving. */
    fun backAsks(): Boolean {
        val now = _state.value
        val next = now.back(twoPane) ?: return false
        return now.composerOpen && !next.composerOpen && composerAsks()
    }

    /** One step back; false when there was nothing to close. Closing the composer goes through [closeComposer]. */
    fun back(): Boolean {
        val now = _state.value
        val next = now.back(twoPane) ?: return false
        if (now.composerOpen && !next.composerOpen) {
            closeComposer()
            return true
        }
        set { next }
        return true
    }

    companion object {
        private const val KEY = "nav"
        private const val POSITIONS = "list_positions"
        private const val MAX_POSITIONS = 40
        private val POSITIONS_SERIALIZER = MapSerializer(String.serializer(), ListSerializer(Int.serializer()))

        // How often a save is repeated while the draft keeps changing under it before the composer just stays open.
        private const val SAVE_TRIES = 3

        // How long a notification tap waits for the accounts on a cold start.
        private const val LOAD_WAIT_MS = 5_000L

        fun factory(services: AppServices): ViewModelProvider.Factory = viewModelFactory {
            initializer { AppViewModel(services, createSavedStateHandle()) }
        }
    }
}
