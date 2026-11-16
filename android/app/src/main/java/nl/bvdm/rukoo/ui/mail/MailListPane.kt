package nl.bvdm.rukoo.ui.mail

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.tween
import androidx.compose.animation.expandHorizontally
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.shrinkHorizontally
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyItemScope
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Text
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.material3.pulltorefresh.PullToRefreshDefaults
import androidx.compose.material3.pulltorefresh.rememberPullToRefreshState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.flowOf
import kotlinx.coroutines.withContext
import nl.bvdm.rukoo.R
import nl.bvdm.rukoo.categories.CategoriesState
import nl.bvdm.rukoo.categories.Category
import nl.bvdm.rukoo.categories.CategoryLookup
import nl.bvdm.rukoo.mail.model.FolderRole
import nl.bvdm.rukoo.mail.model.MailCounts
import nl.bvdm.rukoo.mail.model.MailView
import nl.bvdm.rukoo.mail.model.MessageSummary
import nl.bvdm.rukoo.mail.model.SortOrder
import nl.bvdm.rukoo.mail.model.ViewKind
import nl.bvdm.rukoo.mail.model.QuickFilter as ListFilter
import nl.bvdm.rukoo.settings.Density
import nl.bvdm.rukoo.settings.SwipeAction
import nl.bvdm.rukoo.ui.agent.AgentEntryButton
import nl.bvdm.rukoo.ui.agent.rememberAgentNews
import nl.bvdm.rukoo.ui.components.EmptyState
import nl.bvdm.rukoo.ui.components.Hairline
import nl.bvdm.rukoo.ui.components.QuickFilter
import nl.bvdm.rukoo.ui.components.QuickFilterRow
import nl.bvdm.rukoo.ui.components.RkBottomSheet
import nl.bvdm.rukoo.ui.components.RkButton
import nl.bvdm.rukoo.ui.components.RkButtonKind
import nl.bvdm.rukoo.ui.components.RkButtonSize
import nl.bvdm.rukoo.ui.components.RkDropdownMenu
import nl.bvdm.rukoo.ui.components.RkIcon
import nl.bvdm.rukoo.ui.components.RkIconButton
import nl.bvdm.rukoo.ui.components.RkMenuItem
import nl.bvdm.rukoo.ui.components.RkSearchField
import nl.bvdm.rukoo.ui.components.RkTopBar
import nl.bvdm.rukoo.ui.components.SheetRow
import nl.bvdm.rukoo.ui.components.pressScale
import nl.bvdm.rukoo.ui.components.rkClickable
import nl.bvdm.rukoo.ui.icons.RukooIcons
import nl.bvdm.rukoo.ui.root.AppViewModel
import nl.bvdm.rukoo.ui.root.Pane
import nl.bvdm.rukoo.ui.settings.categoryColor
import nl.bvdm.rukoo.ui.theme.Rk
import nl.bvdm.rukoo.ui.theme.RukooEasing
import nl.bvdm.rukoo.ui.theme.rkShadow
import nl.bvdm.rukoo.ui.theme.semibold

/**
 * The message list (spec 6 §4.2 and §5.2): the title over "N unread · account", the quick filters, rows by
 * day with stacks, selection mode with its bar, swipe actions, pull to refresh, the empty and loading
 * states, the "New message" button and the agent's way in with its news dot. Search replaces the title
 * with a field and lists results from every account.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun MailListPane(app: AppViewModel, modifier: Modifier = Modifier) {
    val c = Rk.colors
    val mail = app.services.mail
    val nav by app.state.collectAsStateWithLifecycle()
    val settings by app.services.settingsSnapshot.collectAsStateWithLifecycle()
    val accounts by mail.accounts.collectAsStateWithLifecycle()
    val folders by mail.folders.collectAsStateWithLifecycle()
    val sync by mail.sync.collectAsStateWithLifecycle()
    val localDrafts by app.services.drafts.drafts.collectAsStateWithLifecycle()
    val dates = rememberMailDates()

    // Typing doesn't run a search per key: the query settles for 200ms first.
    val query by produceState(nav.activeQuery, nav.activeQuery) {
        if (nav.activeQuery.isNotEmpty() && value.isNotEmpty()) delay(200)
        value = nav.activeQuery
    }
    val searching = nav.searching && query.isNotEmpty()
    val listView = if (searching && nav.searchAll) MailView(ViewKind.EVERYTHING) else nav.view
    val flow = remember(listView, query, settings.sort) { mail.messages(listView, query, settings.sort) }
    val loaded by flow.collectAsStateWithLifecycle(initialValue = null)
    val all = loaded.orEmpty()
    val filter = if (nav.searching) ListFilter.ALL else nav.filter

    // Inbox categories sit under Inbox in the folder drawer; the inbox shows the one picked there, while categories
    // are on and nothing is searched (app.js categoriesOn). Every list that holds inbox mail labels it with its
    // category, so each email's category is worked out off the main thread whenever the list, the categories or any
    // record changes.
    val categories = app.services.categories
    val noCategories = remember { MutableStateFlow(CategoriesState()) }
    val catState by (categories?.state ?: noCategories).collectAsStateWithLifecycle()
    val noVersion = remember { MutableStateFlow(0L) }
    val recordsVersion by (app.services.account?.version ?: noVersion).collectAsStateWithLifecycle()
    val categoriesOn = categories != null && catState.enabled && catState.categories.isNotEmpty()
    val tabsOn = categoriesOn && nav.view.kind == ViewKind.INBOX && !nav.searching
    val lookup by produceState(CategoryLookup.NONE, all, catState, recordsVersion, categoriesOn, accounts) {
        value = if (categoriesOn && categories != null) withContext(Dispatchers.Default) { categories.lookup(accounts, all) } else CategoryLookup.NONE
    }
    val byId = remember(catState) { catState.categories.associateBy { it.id } }
    val tab = nav.category?.takeIf { id -> tabsOn && id in byId }
    // The list in the category on screen; the quick filters and their counts work on it.
    val inTab = remember(all, tab, lookup, tabsOn) { ListModel.categoryRows(all, lookup, tab, inbox = tabsOn) }
    // Unread keeps the email opened while unread (nav.keptId) until another opens; then its row leaves (rowMotion).
    val messages = remember(inTab, filter, nav.keptId) { ListModel.applyFilter(inTab, filter, nav.keptId) }
    val byDate = settings.sort == SortOrder.DATE_DESC || settings.sort == SortOrder.DATE_ASC
    val now = remember(all) { System.currentTimeMillis() }
    val rows = remember(messages, byDate, searching, nav.openStacks, nav.openMessageId, nav.checked, dates, now) {
        ListModel.rows(messages, byDate, searching, nav.openStacks, nav.openMessageId, nav.checked) { dates.groupLabel(it, now) }
    }
    val drafts = if (nav.view.kind == ViewKind.DRAFTS && !nav.searching) localDrafts.filter { nav.view.accountId == null || it.accountId == nav.view.accountId } else emptyList()
    LaunchedEffect(rows) { app.listShown(ListModel.visibleIds(rows)) }

    val multi = accounts.size > 1 && (nav.view.accountId == null || (searching && nav.searchAll))
    val labels = remember(accounts) { ListModel.accountLabels(accounts) }
    val showFolder = (searching && nav.searchAll) || nav.view.kind == ViewKind.STARRED
    val checked = nav.checked.toSet()
    val selecting = checked.isNotEmpty()

    // The unread inbox mail older than the messages the list holds, which the drawer's Inbox count adds: Inbox itself
    // names it in its header and explains it under its last row, where a phone has no tooltip (app.js listTitle).
    // A category or a search doesn't.
    val olderShown = nav.view.kind == ViewKind.INBOX && tab == null && !nav.searching
    val countsFlow = remember(mail, nav.view.accountId, olderShown) {
        if (olderShown) runCatching { mail.counts(nav.view.accountId) }.getOrElse { flowOf(MailCounts()) } else flowOf(MailCounts())
    }
    val counts by countsFlow.collectAsStateWithLifecycle(MailCounts())
    val older = if (olderShown) counts.olderUnread else 0
    val olderHint = if (older > 0) pluralStringResource(R.plurals.mailbox_list_older_unread_hint, older, counts.inboxLimit, older) else null

    // One scroll position per list: leaving a folder keeps its place for when it comes back.
    val listKey = "${listView.kind}|${listView.accountId}|${listView.folder}|$searching"
    val listState = rememberSaveable(listKey, saver = LazyListState.Saver) {
        app.listPosition(listKey)?.let { (index, offset) -> LazyListState(index, offset) } ?: LazyListState()
    }
    DisposableEffect(listKey, listState) {
        onDispose { app.keepListPosition(listKey, listState.firstVisibleItemIndex, listState.firstVisibleItemScrollOffset) }
    }
    // Checked emails that left the list (marked read under Unread, moved elsewhere) leave the selection too,
    // so the bar's count and its actions always match what is on screen.
    LaunchedEffect(messages, loaded != null, nav.checked) {
        if (loaded == null || nav.checked.isEmpty()) return@LaunchedEffect
        val here = messages.mapTo(HashSet()) { it.id }
        val keep = nav.checked.filter { it in here }
        if (keep.size != nav.checked.size) app.setChecked(keep)
    }

    Box(modifier.background(c.panel)) {
        Column(Modifier.fillMaxSize()) {
            when {
                selecting -> SelectionBar(app, messages.filter { it.id in checked }, messages)
                nav.searching -> SearchBar(app, nav.query, nav.view, folders)
                else -> ListHeader(app, nav.view, inTab, accounts.map { it.id to it.email }, folders, category = tab?.let(byId::get), older = older)
            }
            if (!selecting) {
                if (nav.searching) {
                    QuickFilterRow(
                        filters = listOf(
                            QuickFilter("all", stringResource(R.string.mailbox_search_all_folders)),
                            QuickFilter("view", stringResource(R.string.mailbox_search_folder_named, viewTitle(nav.view, folders))),
                        ),
                        selectedKey = if (nav.searchAll) "all" else "view",
                        onSelect = { app.setSearchAll(it == "all") },
                    )
                    if (searching && loaded != null) {
                        val scope = if (nav.searchAll) stringResource(R.string.mailbox_search_all_accounts).lowercase(dates.locale) else viewTitle(nav.view, folders)
                        Text(
                            pluralStringResource(R.plurals.mailbox_search_summary, all.size, all.size, query, scope),
                            style = Rk.type.text12,
                            color = c.text3,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                            modifier = Modifier.padding(start = 18.dp, end = 12.dp, top = 6.dp, bottom = 2.dp),
                        )
                    }
                } else {
                    QuickFilterRow(
                        filters = listOf(
                            QuickFilter(ListFilter.ALL.name, stringResource(R.string.mailbox_filters_all)),
                            QuickFilter(ListFilter.UNREAD.name, stringResource(R.string.mailbox_filters_unread), ListModel.filterCount(inTab, ListFilter.UNREAD)),
                            QuickFilter(ListFilter.STARRED.name, stringResource(R.string.mailbox_filters_starred), ListModel.filterCount(inTab, ListFilter.STARRED)),
                            QuickFilter(ListFilter.ATTACHMENTS.name, stringResource(R.string.mailbox_filters_attachments), ListModel.filterCount(inTab, ListFilter.ATTACHMENTS)),
                        ),
                        selectedKey = nav.filter.name,
                        onSelect = { key -> app.setFilter(ListFilter.valueOf(key)) },
                    )
                }
            } else {
                Hairline()
            }

            var pulled by remember { mutableStateOf(false) }
            LaunchedEffect(pulled, sync.syncing) {
                if (pulled && !sync.syncing) {
                    delay(500)
                    pulled = false
                }
            }
            val pullState = rememberPullToRefreshState()
            PullToRefreshBox(
                isRefreshing = pulled,
                onRefresh = {
                    pulled = true
                    app.mailUi.refresh(nav.view.accountId)
                },
                state = pullState,
                modifier = Modifier.weight(1f).fillMaxWidth(),
                indicator = {
                    PullToRefreshDefaults.Indicator(
                        state = pullState,
                        isRefreshing = pulled,
                        modifier = Modifier.align(Alignment.TopCenter),
                        containerColor = c.raised,
                        color = c.text2,
                    )
                },
            ) {
                when {
                    loaded == null -> Box(Modifier.fillMaxSize())
                    rows.isEmpty() && drafts.isEmpty() -> EmptyList(
                        app,
                        searching,
                        nav.searchAll,
                        filter,
                        sync.syncing,
                        sync.errors,
                        accounts.associate { it.id to it.email },
                        emptyTab = tab?.let { byId[it]?.name },
                    )
                    else -> MessageList(
                        app, rows, drafts, listState, dates, now, settings.density == Density.COMPACT, selecting, checked, nav.openMessageId, multi, labels, showFolder, folders,
                        swipeRight = settings.swipeRight.takeIf { settings.swipeActions && it != SwipeAction.NONE },
                        swipeLeft = settings.swipeLeft.takeIf { settings.swipeActions && it != SwipeAction.NONE },
                        categoryOf = { m -> ListModel.rowCategory(m, lookup, tab, byId) },
                        footer = olderHint,
                    )
                }
            }
        }
        if (!selecting && !nav.searching) {
            FoldersButton(onClick = app::openDrawer, modifier = Modifier.align(Alignment.BottomStart).padding(16.dp))
            NewMessageButton(
                expanded = !listState.lastScrolledForward || !listState.canScrollBackward,
                onClick = { app.newMessage() },
                modifier = Modifier.align(Alignment.BottomEnd).padding(16.dp),
            )
        }
    }
}

@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun MessageList(
    app: AppViewModel,
    rows: List<ListRow>,
    drafts: List<nl.bvdm.rukoo.ui.ComposerState>,
    state: LazyListState,
    dates: MailDates,
    now: Long,
    compact: Boolean,
    selecting: Boolean,
    checked: Set<String>,
    openId: String?,
    multi: Boolean,
    labels: Map<String, String>,
    showFolder: Boolean,
    folders: Map<String, List<nl.bvdm.rukoo.mail.model.Folder>>,
    swipeRight: SwipeAction?,
    swipeLeft: SwipeAction?,
    /** The category label a row shows, if any (ListModel.rowCategory). */
    categoryOf: (MessageSummary) -> Category?,
    /** A note under the last row, such as why older unread mail isn't listed. */
    footer: String? = null,
) {
    val c = Rk.colors
    val ui = app.mailUi
    val sentTo = stringResource(R.string.mailbox_list_sent_to, "%s")
    val noRecipient = stringResource(R.string.mailbox_message_no_recipient)
    val unknown = stringResource(R.string.mailbox_message_unknown_sender)
    val folderNames = folders.mapValues { (_, list) -> list.associate { it.path to folderName(it) } }
    val reduced = Rk.motion.reduced
    LazyColumn(
        state = state,
        modifier = Modifier.fillMaxSize(),
        // Room under the last row for the New message button.
        contentPadding = PaddingValues(top = 2.dp, bottom = 88.dp),
    ) {
        if (drafts.isNotEmpty()) {
            stickyHeader(key = "head:local") { GroupHead(stringResource(R.string.mail_drafts_local)) }
            items(drafts, key = { "draft:${it.id}" }) { d ->
                val names = d.to.map { ListModel.person(it) }.filter { it.isNotEmpty() }
                LocalDraftRow(
                    d,
                    if (names.isEmpty()) noRecipient else sentTo.replace("%s", names.joinToString(", ")),
                    if (d.savedAt > 0) dates.listTime(d.savedAt, now) else "",
                    onOpen = { app.openLocalDraft(d.id) },
                    modifier = rowMotion(reduced),
                )
            }
        }
        for (row in rows) {
            when (row) {
                is ListRow.Head -> stickyHeader(key = row.key) { GroupHead(row.label) }
                is ListRow.Mail -> item(key = row.key) {
                    val m = row.message
                    MessageRow(
                        m = m,
                        line = ListModel.senderLine(m, { sentTo.replace("%s", it) }, noRecipient, unknown),
                        time = dates.listTime(m.date, now),
                        look = RowLook(
                            compact = compact,
                            selecting = selecting,
                            open = m.id == openId,
                            checked = m.id in checked,
                            accountLabel = if (multi) labels[m.accountId] else null,
                            accountColor = m.accountColor,
                            folderLabel = if (showFolder) folderNames[m.accountId]?.get(m.folder) ?: m.folder else null,
                            category = categoryOf(m),
                        ),
                        actions = RowActions(
                            open = {
                                app.interacted(Pane.LIST)
                                app.openMessage(m.id)
                            },
                            toggleChecked = { app.toggleChecked(m.id) },
                            right = swipeRight?.let { rowSwipe(ui, it, m) },
                            left = swipeLeft?.let { rowSwipe(ui, it, m) },
                            toggleStack = row.stack?.let { s -> { app.toggleStack(s.key) } },
                        ),
                        stack = row.stack,
                        stacked = row.stacked,
                        modifier = rowMotion(reduced),
                    )
                }
            }
        }
        if (footer != null) {
            item(key = "footer") {
                Text(
                    footer,
                    style = Rk.type.text12,
                    color = c.text3,
                    modifier = Modifier.fillMaxWidth().padding(start = 20.dp, end = 20.dp, top = 16.dp, bottom = 8.dp),
                )
            }
        }
    }
}

/**
 * How a row comes, moves and goes. One that leaves, such as the email the Unread filter kept once another opens, fades
 * out where it is while the rows below close the gap (app.js letGo):
 *     0ms  the row fades out
 *   110ms  it is gone; the rows below go on moving up into its place
 * With the system's animations removed, rows don't animate.
 */
private fun LazyItemScope.rowMotion(reduced: Boolean): Modifier =
    if (reduced) Modifier.animateItem(fadeInSpec = null, placementSpec = null, fadeOutSpec = null)
    else Modifier.animateItem(fadeOutSpec = tween(LEAVE_FADE_MS, easing = RukooEasing.Out))

// How long a leaving row takes to fade (app.js letGo's first step).
private const val LEAVE_FADE_MS = 110

/**
 * What a swipe with [action] does to [m], or null where it can't: a saved or draft email only goes away, and only
 * inbox mail has a category (desktop's rowActionAvailable).
 */
private fun rowSwipe(ui: MailUi, action: SwipeAction, m: MessageSummary): RowSwipe? {
    val one = listOf(m)
    val run: () -> Unit = when (action) {
        SwipeAction.NONE -> return null
        SwipeAction.DELETE -> ({ ui.delete(one) })
        else -> {
            if (m.role == FolderRole.DRAFTS && action != SwipeAction.STAR) return null
            when (action) {
                SwipeAction.READ -> ({ ui.setUnread(one, !m.unread) })
                SwipeAction.ARCHIVE -> {
                    if (m.role == FolderRole.ARCHIVE) return null
                    ({ ui.archive(one) })
                }
                SwipeAction.STAR -> ({ ui.setStarred(one, !m.starred) })
                SwipeAction.MOVE -> ({ ui.pickFolder(one) })
                SwipeAction.CATEGORY -> {
                    if (m.id !in ui.categoriesOf(one)) return null
                    ({ ui.pickCategory(one) })
                }
                else -> return null
            }
        }
    }
    return RowSwipe(action, run)
}

/** A sticky day head (styles.css:1222-1231): 11.5/600 `text3` on `panel`. */
@Composable
private fun GroupHead(label: String) {
    Text(
        label,
        style = Rk.type.text11_5.semibold,
        color = Rk.colors.text3,
        maxLines = 1,
        modifier = Modifier.fillMaxWidth().background(Rk.colors.panel).padding(start = 20.dp, end = 12.dp, top = 12.dp, bottom = 4.dp),
    )
}

/**
 * "N unread · account" when the view counts unread mail, else the account or "All accounts" (app.js listTitle). A
 * category under Inbox titles the list with its name and wears its color, so it can't be taken for Inbox itself; the
 * line below names the account or "All accounts" too. Inbox names its [older] unread mail apart, the mail older than
 * the messages the list holds, where the list's own unread mail is counted.
 */
@Composable
private fun ListHeader(
    app: AppViewModel,
    view: MailView,
    list: List<MessageSummary>,
    accounts: List<Pair<String, String>>,
    folders: Map<String, List<nl.bvdm.rukoo.mail.model.Folder>>,
    category: Category? = null,
    older: Int = 0,
) {
    val c = Rk.colors
    val where = view.accountId?.let { id -> accounts.firstOrNull { it.first == id }?.second }
        ?: accounts.singleOrNull()?.second
        ?: stringResource(R.string.mailbox_search_all_accounts)
    val counts = view.kind in setOf(ViewKind.INBOX, ViewKind.FOLDER, ViewKind.VIP, ViewKind.JUNK, ViewKind.ARCHIVE, ViewKind.UNREAD)
    val unread = if (counts) list.count { it.unread } else 0
    val sub = when {
        older > 0 && unread > 0 -> stringResource(R.string.mailbox_list_unread_summary_older, unread, older, where)
        older > 0 -> stringResource(R.string.mailbox_list_older_unread_summary, older, where)
        unread > 0 -> stringResource(R.string.mailbox_list_unread_summary, unread, where)
        else -> where
    }
    var menu by remember { mutableStateOf(false) }
    var sort by remember { mutableStateOf(false) }
    val settings by app.services.settingsSnapshot.collectAsStateWithLifecycle()
    val news = rememberAgentNews(app)
    RkTopBar(
        title = category?.name ?: viewTitle(view, folders),
        subtitle = sub,
        titleStyle = Rk.type.pageTitle,
        height = 64.dp,
        hairline = false,
        tint = category?.let { categoryColor(it.color) },
        // The folders open from FoldersButton at the bottom, where the thumb is, rather than from a menu
        // button in the top corner.
        actions = {
            RkIconButton(RukooIcons.Search, stringResource(R.string.mailbox_search_label), onClick = app::openSearch)
            // The chat UI's entry: a chat about the selected email, with the news dot; the news itself is in the
            // chat's bell and under More.
            AgentEntryButton(app, emailId = null)
            Box {
                RkIconButton(RukooIcons.More, stringResource(R.string.common_actions_more_options), onClick = { menu = true })
                RkDropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
                    RkMenuItem(stringResource(R.string.mailbox_actions_read_all), icon = RukooIcons.MailOpen, onClick = {
                        menu = false
                        app.mailUi.markAllRead(view, category?.id)
                    })
                    RkMenuItem(stringResource(R.string.mailbox_actions_sync), icon = RukooIcons.Sync, onClick = {
                        menu = false
                        app.mailUi.refresh(view.accountId)
                    })
                    RkMenuItem(
                        stringResource(R.string.mailbox_actions_compact),
                        icon = RukooIcons.Ul,
                        checked = settings.density == Density.COMPACT,
                        onClick = {
                            menu = false
                            app.setDensity(if (settings.density == Density.COMPACT) Density.STANDARD else Density.COMPACT)
                        },
                    )
                    RkMenuItem(stringResource(R.string.mailbox_sort_menu), icon = RukooIcons.ChevronUpDown, onClick = {
                        menu = false
                        sort = true
                    })
                    RkMenuItem(stringResource(R.string.mail_agent_history), icon = RukooIcons.Bell, tag = news.count.takeIf { it > 0 }?.toString(), onClick = {
                        menu = false
                        app.openChatList()
                    })
                    RkMenuItem(stringResource(R.string.mail_drawer_settings), icon = RukooIcons.Settings, onClick = {
                        menu = false
                        app.openSettings()
                    })
                }
            }
        },
    )
    if (sort) {
        RkBottomSheet(onDismissRequest = { sort = false }, title = stringResource(R.string.mailbox_sort_title)) {
            for ((order, label) in listOf(
                SortOrder.DATE_DESC to R.string.mailbox_sort_newest,
                SortOrder.DATE_ASC to R.string.mailbox_sort_oldest,
                SortOrder.UNREAD to R.string.mailbox_sort_unread,
                SortOrder.SENDER to R.string.mailbox_sort_sender,
            )) {
                SheetRow(stringResource(label), checked = settings.sort == order, onClick = {
                    app.setSort(order)
                    dismiss()
                })
            }
        }
    }
}

@Composable
private fun SearchBar(app: AppViewModel, query: String, view: MailView, folders: Map<String, List<nl.bvdm.rukoo.mail.model.Folder>>) {
    val focus = remember { FocusRequester() }
    Row(
        Modifier.fillMaxWidth().heightIn(min = 64.dp).background(Rk.colors.panel).padding(start = 4.dp, end = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        RkIconButton(RukooIcons.Back, stringResource(R.string.common_actions_back), onClick = app::closeSearch)
        Spacer(Modifier.width(4.dp))
        Box(Modifier.weight(1f)) {
            RkSearchField(
                value = query,
                onValueChange = app::setQuery,
                placeholder = stringResource(R.string.mailbox_search_placeholder, viewTitle(view, folders)),
                focusRequester = focus,
            )
        }
    }
    LaunchedEffect(Unit) { if (query.isEmpty()) runCatching { focus.requestFocus() } }
}

/**
 * The selection bar (app.js renderListTools, narrow form): the tri-state checkbox and count, then Read or
 * Unread, Delete, More (Star, Move, Archive, Add to chat) and Clear, on `accentSoft`.
 */
@Composable
private fun SelectionBar(app: AppViewModel, picked: List<MessageSummary>, visible: List<MessageSummary>) {
    val c = Rk.colors
    val ui = app.mailUi
    val all = visible.isNotEmpty() && visible.all { m -> picked.any { it.id == m.id } }
    val anyUnread = picked.any { it.unread }
    var more by remember { mutableStateOf(false) }
    Row(
        Modifier.fillMaxWidth().heightIn(min = 64.dp).background(c.accentSoft).padding(start = 8.dp, end = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(
            Modifier
                .heightIn(min = 48.dp)
                .width(48.dp)
                .rkClickable(Rk.shapes.control, onClickLabel = stringResource(if (all) R.string.mailbox_selection_none else R.string.mailbox_selection_all)) {
                    if (all) app.clearChecked() else app.setChecked(visible.map { it.id })
                },
            contentAlignment = Alignment.Center,
        ) { MailCheckbox(all, partial = !all) }
        Text(
            stringResource(R.string.mailbox_selection_count, picked.size),
            style = Rk.type.text14.semibold,
            color = c.text,
            maxLines = 1,
            modifier = Modifier.weight(1f).padding(start = 4.dp),
        )
        RkIconButton(
            if (anyUnread) RukooIcons.MailOpen else RukooIcons.MarkUnread,
            stringResource(if (anyUnread) R.string.mailbox_actions_read else R.string.mailbox_actions_unread),
            onClick = { ui.setUnread(picked, !anyUnread) },
        )
        RkIconButton(RukooIcons.Trash, stringResource(R.string.common_actions_delete), onClick = { ui.delete(picked) })
        RkIconButton(RukooIcons.More, stringResource(R.string.reader_actions_more), onClick = { more = true })
        RkIconButton(RukooIcons.Close, stringResource(R.string.mailbox_selection_clear), onClick = app::clearChecked)
    }
    if (more) {
        val canArchive = picked.any { app.services.mail.archiveFolder(it.accountId) != null && it.role != FolderRole.ARCHIVE && it.role != FolderRole.ALL }
        val starred = picked.isNotEmpty() && picked.all { it.starred }
        // Move to category, for inbox mail while categories are on (desktop shows it when one has a category).
        val inCategories = remember(picked) { ui.categoriesOf(picked).isNotEmpty() }
        RkBottomSheet(onDismissRequest = { more = false }, title = pluralStringResource(R.plurals.mailbox_selection_summary, picked.size, picked.size)) {
            SheetRow(stringResource(if (starred) R.string.mailbox_actions_unstar else R.string.mailbox_actions_add_star), icon = if (starred) RukooIcons.StarFilled else RukooIcons.Star, onClick = {
                ui.setStarred(picked, !starred)
                dismiss()
            })
            SheetRow(stringResource(R.string.mail_agent_add_to_chat), icon = RukooIcons.Sparkle, onClick = {
                dismiss()
                app.addToChat(picked.map { it.id })
            })
            SheetRow(stringResource(R.string.mailbox_actions_move_menu), icon = RukooIcons.Move, onClick = {
                dismiss()
                ui.pickFolder(picked)
            })
            if (inCategories) {
                SheetRow(stringResource(R.string.mailbox_categories_move), icon = RukooIcons.Inbox, onClick = {
                    dismiss()
                    ui.pickCategory(picked)
                })
            }
            if (canArchive) {
                SheetRow(stringResource(R.string.common_actions_archive), icon = RukooIcons.Archive, onClick = {
                    dismiss()
                    ui.archive(picked)
                })
            }
            SheetRow(pluralStringResource(R.plurals.mailbox_delete_count_action, picked.size, picked.size), icon = RukooIcons.Trash, danger = true, onClick = {
                dismiss()
                ui.delete(picked)
            })
        }
    }
}

/** The empty list (app.js listHtml): no mail, no results with a hint, nothing in the filter with Show all, or syncing. */
@Composable
private fun EmptyList(
    app: AppViewModel,
    searching: Boolean,
    searchAll: Boolean,
    filter: ListFilter,
    syncing: Boolean,
    errors: Map<String, String>,
    emails: Map<String, String>,
    /** The category tab on screen, when it is the one that is empty. */
    emptyTab: String? = null,
) {
    val c = Rk.colors
    androidx.compose.foundation.lazy.LazyColumn(Modifier.fillMaxSize()) {
        item {
            Column(Modifier.fillMaxWidth().padding(top = 22.dp), horizontalAlignment = Alignment.CenterHorizontally) {
                when {
                    searching -> {
                        EmptyState(
                            stringResource(R.string.mailbox_empty_search),
                            hint = stringResource(if (searchAll) R.string.mailbox_empty_search_other else R.string.mailbox_empty_search_all),
                            icon = RukooIcons.Search,
                        )
                        if (!searchAll) RkButton(stringResource(R.string.mailbox_empty_search_all_action), { app.setSearchAll(true) }, kind = RkButtonKind.SECONDARY, size = RkButtonSize.SMALL)
                    }
                    filter == ListFilter.ALL && emptyTab != null -> EmptyState(stringResource(R.string.mailbox_categories_empty, emptyTab))
                    filter != ListFilter.ALL -> EmptyState(
                        stringResource(R.string.mailbox_empty_filter),
                        actionLabel = stringResource(R.string.mailbox_empty_show_all),
                        onAction = { app.setFilter(ListFilter.ALL) },
                    )
                    syncing -> EmptyState(stringResource(R.string.mailbox_empty_syncing))
                    else -> EmptyState(stringResource(R.string.mailbox_empty_folder))
                }
                if (!syncing) {
                    errors.entries.firstOrNull()?.let { (id, message) ->
                        Text(
                            "${emails[id] ?: id}: $message",
                            style = Rk.type.text13,
                            color = c.danger,
                            modifier = Modifier.padding(horizontal = 24.dp, vertical = 8.dp),
                        )
                    }
                }
            }
        }
    }
}

/**
 * "New message", styled like desktop's compose button (styles.css:696-709): a raised card with the compose
 * icon in `accent` and the label at 600. It folds to the icon while the list scrolls down.
 */
/** The folders, in the bottom corner across from New message, in the same raised style. */
@Composable
private fun FoldersButton(onClick: () -> Unit, modifier: Modifier = Modifier) {
    val c = Rk.colors
    val shape = Rk.shapes.panel
    val source = remember { MutableInteractionSource() }
    Box(
        modifier
            .pressScale(source, 0.96f)
            .rkShadow(Rk.elevation.raised, shape)
            .background(c.raised, shape)
            .rkClickable(shape, interactionSource = source, onClick = onClick)
            .size(48.dp),
        contentAlignment = Alignment.Center,
    ) {
        RkIcon(RukooIcons.Menu, stringResource(R.string.mailbox_layout_folders), size = 20.dp, tint = c.text2)
    }
}

@Composable
private fun NewMessageButton(expanded: Boolean, onClick: () -> Unit, modifier: Modifier = Modifier) {
    val c = Rk.colors
    val shape = Rk.shapes.panel
    val source = remember { MutableInteractionSource() }
    Row(
        modifier
            .pressScale(source, 0.96f)
            .rkShadow(Rk.elevation.raised, shape)
            .background(c.raised, shape)
            .rkClickable(shape, interactionSource = source, onClick = onClick)
            .heightIn(min = 48.dp)
            .padding(horizontal = 14.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        RkIcon(RukooIcons.Compose, stringResource(R.string.composer_titles_new).takeIf { !expanded }, size = 20.dp, tint = c.accent)
        AnimatedVisibility(expanded, enter = fadeIn() + expandHorizontally(), exit = fadeOut() + shrinkHorizontally()) {
            Row {
                Spacer(Modifier.width(10.dp))
                Text(stringResource(R.string.composer_titles_new), style = Rk.type.text13.semibold, color = c.text, maxLines = 1)
                Spacer(Modifier.width(2.dp))
            }
        }
    }
}
