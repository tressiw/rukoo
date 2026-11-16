package nl.bvdm.rukoo.ui.mail

import nl.bvdm.rukoo.categories.Category
import nl.bvdm.rukoo.categories.CategoryLookup
import nl.bvdm.rukoo.mail.model.Account
import nl.bvdm.rukoo.mail.model.Address
import nl.bvdm.rukoo.mail.model.FolderRole
import nl.bvdm.rukoo.mail.model.MessageSummary
import nl.bvdm.rukoo.mail.model.QuickFilter as ListFilter

/** A folded run of mail from one sender (app.js stackLayout): [count] includes the head row. */
data class StackInfo(val key: String, val count: Int, val open: Boolean, val forced: Boolean, val unread: Boolean)

/** What the message list draws, in order. */
sealed interface ListRow {
    val key: String

    /** A sticky day head ("Today", "Yesterday", ...). */
    data class Head(val label: String) : ListRow {
        override val key: String get() = "head:$label"
    }

    /** A message row; [stack] on a stack's head, [stacked] on an open stack's other members. */
    data class Mail(val message: MessageSummary, val stack: StackInfo? = null, val stacked: Boolean = false) : ListRow {
        override val key: String get() = message.id
    }
}

/** The message list's pure logic, ported from app.js (renderList, stackLayout, listTitle) and ui.js (cleanPreview). */
object ListModel {
    // Newsletters open with "view in browser" lines and mail with a greeting; neither says what it is about (ui.js:97-104).
    private val PREVIEW_NOISE = listOf(
        Regex(
            "^(bekijk|lees|open|view|read)( (deze|de|het|this|the|onze|our))?( (e-?mail|mail|nieuwsbrief|newsletter|bericht|message))?" +
                "( (in|online|op|on))?( (je|jouw|uw|de|een|your|a|the))? ?(web)?(browser|versie|version|site|website)\\b[.!:]?\\s*",
            RegexOption.IGNORE_CASE,
        ),
        Regex("^(web ?versie|online versie|web version|view online|online bekijken)\\b[.!:]?\\s*", RegexOption.IGNORE_CASE),
        Regex(
            "^(problemen met (het )?(weergeven|bekijken|lezen)[^.?!]*[.?!]|(wordt|is) deze (e-?mail|nieuwsbrief) niet goed " +
                "(weergegeven|leesbaar|zichtbaar)[^.?!]*[.?!]?|having trouble (viewing|reading)[^.?!]*[.?!]?|" +
                "can'?t see (this|the) (e-?mail|images)[^.?!]*[.?!]?)\\s*",
            RegexOption.IGNORE_CASE,
        ),
        Regex("^(beste|hoi|hallo|hi|hey|hello|dear|geachte|goedemorgen|goedemiddag|goedenavond)\\b[^,!\\n]{0,40}[,!]\\s*", RegexOption.IGNORE_CASE),
    )
    // Unicode spaces too (newsletters are full of NBSP). Android's ICU regex has no (?U) flag, so they are listed.
    private val SPACES = Regex("[\\s\\u00A0\\u1680\\u2000-\\u200B\\u2028\\u2029\\u202F\\u205F\\u3000\\uFEFF]+")

    /** Up to four passes over the noise; when everything goes, the raw text stays. */
    fun cleanPreview(text: String): String {
        var out = SPACES.replace(text, " ").trim()
        repeat(4) {
            val before = out
            for (re in PREVIEW_NOISE) out = re.replaceFirst(out, "")
            if (out == before) return out.ifEmpty { text.trim() }
        }
        return out.ifEmpty { text.trim() }
    }

    /** [list] under a quick filter. Unread also keeps [keptId], the email opened while unread (app.js rows). */
    fun applyFilter(list: List<MessageSummary>, filter: ListFilter, keptId: String? = null): List<MessageSummary> = when (filter) {
        ListFilter.ALL -> list
        ListFilter.UNREAD -> list.filter { it.unread || it.id == keptId }
        ListFilter.STARRED -> list.filter { it.starred }
        ListFilter.ATTACHMENTS -> list.filter { it.hasAttachments }
    }

    /** Counts after each quick filter's label, over the loaded list. Unread counts only unread mail, without the kept email. */
    fun filterCount(list: List<MessageSummary>, filter: ListFilter): Int = if (filter == ListFilter.ALL) 0 else applyFilter(list, filter).size

    /**
     * The list in the category tab on screen (app.js categoryRows): a category's [tab] shows its own mail. Inbox itself
     * ([inbox]: the inbox with nothing searched) leaves out the categories whose Show in Inbox is off; search, VIP and
     * Unread still show them.
     */
    fun categoryRows(list: List<MessageSummary>, lookup: CategoryLookup, tab: String?, inbox: Boolean = false): List<MessageSummary> = when {
        tab != null -> list.filter { lookup[it] == tab }
        inbox && lookup.hiddenCategories.isNotEmpty() -> list.filterNot(lookup::hidden)
        else -> list
    }

    /** Unread mail in Inbox for its count in the folder drawer: [unread] (the Unread view's mail) without hidden categories. */
    fun inboxUnread(unread: List<MessageSummary>, lookup: CategoryLookup): Int = unread.count { !lookup.hidden(it) }

    /**
     * The category label a row shows (app.js rowCategory): an inbox email's category in a list that mixes them, such as
     * Inbox, a search, VIP or Unread. A category's own [tab] says which it is in its header, so its rows have none.
     */
    fun rowCategory(m: MessageSummary, lookup: CategoryLookup, tab: String?, categories: Map<String, Category>): Category? =
        if (tab != null) null else lookup[m]?.let(categories::get)

    /**
     * How wide each pill on a row's subject line is (styles.css .flags): the width it needs, [basis], while they all fit
     * in [room]; else each gives up room in proportion to its width, as CSS flex-shrink does, so a long label ends in an
     * ellipsis before a short one does.
     */
    fun pillWidths(basis: List<Int>, room: Int): List<Int> {
        val total = basis.sum()
        if (total <= room) return basis
        if (total == 0 || room <= 0) return basis.map { 0 }
        return basis.map { (it.toLong() * room / total).toInt() }
    }

    /** Sent mail and drafts never stack; other mail stacks by sender address. */
    fun stackKey(m: MessageSummary): String? {
        if (m.role == FolderRole.SENT || m.role == FolderRole.DRAFTS) return null
        return m.from.address.lowercase().ifEmpty { null }
    }

    /**
     * The rows for [list] (already filtered and sorted). With [byDate] the rows get day heads from
     * [dayLabel], and three or more consecutive messages from one sender on one day fold into a stack,
     * unless [searching]. A stack holding the open or a checked message stays open.
     */
    fun rows(
        list: List<MessageSummary>,
        byDate: Boolean,
        searching: Boolean,
        openStacks: Collection<String>,
        openId: String?,
        checked: Collection<String>,
        dayLabel: (Long) -> String,
    ): List<ListRow> {
        val heads = HashMap<String, StackInfo>()
        val hidden = HashSet<String>()
        val members = HashSet<String>()
        val labels = list.map { if (byDate) dayLabel(it.date) else "" }
        if (byDate && !searching) {
            var i = 0
            while (i < list.size) {
                val key = stackKey(list[i])
                var j = i + 1
                if (key != null) while (j < list.size && stackKey(list[j]) == key && labels[j] == labels[i]) j++
                val rest = list.subList(i + 1, j)
                if (key != null && rest.size >= 2) {
                    val id = "${labels[i]}|$key"
                    val forced = rest.any { it.id == openId || it.id in checked }
                    val open = forced || id in openStacks
                    heads[list[i].id] = StackInfo(id, rest.size + 1, open, forced, rest.any { it.unread })
                    for (m in rest) (if (open) members else hidden) += m.id
                }
                i = j
            }
        }
        val out = ArrayList<ListRow>(list.size + 8)
        var label: String? = null
        list.forEachIndexed { index, m ->
            if (m.id in hidden) return@forEachIndexed
            if (byDate && labels[index] != label) {
                label = labels[index]
                out += ListRow.Head(labels[index])
            }
            out += ListRow.Mail(m, heads[m.id], m.id in members)
        }
        return out
    }

    /** The messages on screen, in order: what Previous and Next walk through, and where Archive goes on to. */
    fun visibleIds(rows: List<ListRow>): List<String> = rows.mapNotNull { (it as? ListRow.Mail)?.message?.id }

    /** The message to open after [gone] left the list: the next one down, else the one above (app.js detach). */
    fun nextAfter(visible: List<String>, open: String?, gone: Collection<String>): String? {
        if (open == null || open !in gone) return null
        val i = visible.indexOf(open)
        if (i < 0) return null
        return visible.drop(i + 1).firstOrNull { it !in gone } ?: visible.take(i).reversed().firstOrNull { it !in gone }
    }

    /** `from.name || from.address`. */
    fun person(a: Address?): String = a?.let { it.name.trim().ifEmpty { it.address } }.orEmpty()

    /**
     * The sender line (app.js senderLine): sent mail and drafts show "To: names" or "(No recipient)", other
     * mail the sender or "(Unknown sender)".
     */
    fun senderLine(m: MessageSummary, sentTo: (String) -> String, noRecipient: String, unknown: String): String {
        if (m.role == FolderRole.SENT || m.role == FolderRole.DRAFTS) {
            val names = m.to.map(::person).filter { it.isNotEmpty() }
            return if (names.isNotEmpty()) sentTo(names.joinToString(", ")) else noRecipient
        }
        return person(m.from).ifEmpty { unknown }
    }

    /** The address an avatar stands for: the first recipient for sent mail and drafts, else the sender. */
    fun avatarOf(m: MessageSummary): Address =
        if (m.role == FolderRole.SENT || m.role == FolderRole.DRAFTS) m.to.firstOrNull() ?: m.from else m.from

    /**
     * The shortest label that still tells the accounts apart (app.js accountLabel): the domain without its
     * TLD, the whole domain, the name before the @, or the address.
     */
    fun accountLabels(accounts: List<Account>): Map<String, String> {
        fun domain(a: Account) = a.email.substringAfter('@', a.email)
        val candidates = listOf(
            accounts.map { domain(it).replace(Regex("\\.[^.]+$"), "") },
            accounts.map { domain(it) },
            accounts.map { it.email.substringBefore('@') },
            accounts.map { it.email },
        )
        val pick = candidates.firstOrNull { list -> list.map { it.lowercase() }.toSet().size == list.size } ?: candidates[3]
        return accounts.mapIndexed { i, a -> a.id to pick[i] }.toMap()
    }
}
