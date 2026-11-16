package nl.bvdm.rukoo.ui.mail

import nl.bvdm.rukoo.categories.Category
import nl.bvdm.rukoo.categories.CategoryLookup
import nl.bvdm.rukoo.mail.model.Account
import nl.bvdm.rukoo.mail.model.AccountType
import nl.bvdm.rukoo.mail.model.Address
import nl.bvdm.rukoo.mail.model.AuthKind
import nl.bvdm.rukoo.mail.model.FolderRole
import nl.bvdm.rukoo.mail.model.MessageSummary
import nl.bvdm.rukoo.mail.model.QuickFilter
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class ListModelTest {
    private fun msg(id: String, from: String, day: Int, unread: Boolean = false, role: FolderRole = FolderRole.INBOX, to: List<Address> = emptyList()) =
        MessageSummary(id = id, accountId = "a", folder = "INBOX", role = role, uid = 1, from = Address(from.substringBefore('@'), from), date = day.toLong(), unread = unread, to = to)

    @Test
    fun previewsLoseBrowserLinesAndGreetings() {
        assertEquals("Shall we have a call?", ListModel.cleanPreview("Hi Sanne, Shall we have a call?"))
        assertEquals("Driving in low sun", ListModel.cleanPreview("View email in browser  Driving in low sun"))
        assertEquals("Nieuws van deze week", ListModel.cleanPreview("Bekijk deze e-mail in je browser. Nieuws van deze week"))
        assertEquals("Uw factuur", ListModel.cleanPreview("Beste klant, Uw factuur"))
        // Nothing left: the raw text stays.
        assertEquals("Hi Sanne,", ListModel.cleanPreview("Hi Sanne,"))
        assertEquals("a b", ListModel.cleanPreview("a \n b"))
    }

    @Test
    fun quickFiltersAndCounts() {
        val list = listOf(msg("1", "a@x.nl", 1, unread = true), msg("2", "b@x.nl", 1).copy(starred = true, hasAttachments = true))
        assertEquals(listOf("1"), ListModel.applyFilter(list, QuickFilter.UNREAD).map { it.id })
        assertEquals(listOf("2"), ListModel.applyFilter(list, QuickFilter.STARRED).map { it.id })
        assertEquals(1, ListModel.filterCount(list, QuickFilter.ATTACHMENTS))
        assertEquals(0, ListModel.filterCount(list, QuickFilter.ALL))
    }

    @Test
    fun unreadKeepsTheEmailOpenedWhileUnreadButDoesNotCountIt() {
        val list = listOf(msg("1", "a@x.nl", 3, unread = true), msg("2", "b@x.nl", 2), msg("3", "c@x.nl", 1))
        assertEquals(listOf("1", "2"), ListModel.applyFilter(list, QuickFilter.UNREAD, keptId = "2").map { it.id })
        assertEquals(listOf("1"), ListModel.applyFilter(list, QuickFilter.UNREAD, keptId = null).map { it.id })
        // Only Unread keeps it.
        assertEquals(emptyList<String>(), ListModel.applyFilter(list, QuickFilter.STARRED, keptId = "2").map { it.id })
        assertEquals(1, ListModel.filterCount(list, QuickFilter.UNREAD))
    }

    @Test
    fun threeFromOneSenderOnOneDayFoldIntoAStack() {
        val list = listOf(
            msg("1", "noreply@vb.nl", 5),
            msg("2", "noreply@vb.nl", 5, unread = true),
            msg("3", "noreply@vb.nl", 5),
            msg("4", "sanne@x.nl", 5),
            msg("5", "noreply@vb.nl", 4),
        )
        val rows = ListModel.rows(list, byDate = true, searching = false, openStacks = emptyList(), openId = null, checked = emptyList()) { "day$it" }
        assertEquals(listOf("head:day5", "1", "4", "head:day4", "5"), rows.map { it.key })
        val head = (rows[1] as ListRow.Mail).stack!!
        assertEquals(3, head.count)
        assertTrue(head.unread)
        assertFalse(head.open)
        // Open: the members follow, marked as stacked.
        val open = ListModel.rows(list, true, false, listOf(head.key), null, emptyList()) { "day$it" }
        assertEquals(listOf("head:day5", "1", "2", "3", "4", "head:day4", "5"), open.map { it.key })
        assertTrue((open[2] as ListRow.Mail).stacked)
        // A checked member keeps it open; searching and other sorts never stack.
        assertEquals(7, ListModel.rows(list, true, false, emptyList(), null, listOf("3")) { "day$it" }.size)
        assertEquals(5, ListModel.rows(list, false, false, emptyList(), null, emptyList()) { "" }.size)
        assertEquals(7, ListModel.rows(list, true, true, emptyList(), null, emptyList()) { "day$it" }.size)
    }

    @Test
    fun sentMailNeverStacks() {
        val list = (1..3).map { msg("$it", "me@x.nl", 1, role = FolderRole.SENT) }
        val rows = ListModel.rows(list, true, false, emptyList(), null, emptyList()) { "d" }
        assertTrue(rows.filterIsInstance<ListRow.Mail>().all { it.stack == null })
    }

    @Test
    fun theReaderGoesOnToTheNextEmail() {
        val shown = listOf("a", "b", "c")
        assertEquals("c", ListModel.nextAfter(shown, "b", listOf("b")))
        assertEquals("b", ListModel.nextAfter(shown, "c", listOf("c")))
        assertNull(ListModel.nextAfter(shown, "a", listOf("a", "b", "c")))
        assertNull(ListModel.nextAfter(shown, "a", listOf("b")))
    }

    @Test
    fun senderLines() {
        val sent = msg("1", "me@x.nl", 1, role = FolderRole.SENT, to = listOf(Address("Joris Bakker", "joris@x.nl"), Address("", "eva@x.nl")))
        assertEquals("To: Joris Bakker, eva@x.nl", ListModel.senderLine(sent, { "To: $it" }, "(No recipient)", "(Unknown sender)"))
        assertEquals("(No recipient)", ListModel.senderLine(sent.copy(to = emptyList()), { "To: $it" }, "(No recipient)", "(Unknown sender)"))
        assertEquals("(Unknown sender)", ListModel.senderLine(msg("2", "", 1), { it }, "", "(Unknown sender)"))
    }

    @Test
    fun accountLabelsAreTheShortestThatTellThemApart() {
        fun acc(id: String, email: String) = Account(id, AccountType.IMAP, "other", AuthKind.PASSWORD, email, "")
        assertEquals(mapOf("1" to "gmail", "2" to "bvdm"), ListModel.accountLabels(listOf(acc("1", "bart@gmail.com"), acc("2", "bart@bvdm.ai"))))
        assertEquals(mapOf("1" to "gmail.com", "2" to "gmail.nl"), ListModel.accountLabels(listOf(acc("1", "a@gmail.com"), acc("2", "b@gmail.nl"))))
        assertEquals(mapOf("1" to "a", "2" to "b"), ListModel.accountLabels(listOf(acc("1", "a@gmail.com"), acc("2", "b@gmail.com"))))
    }

    private fun category(id: String) = Category(id, id, id.replaceFirstChar { it.uppercase() }, "", false, false, "#c56cf0", 0)

    @Test
    fun inboxShowsEachEmailItsCategoryAndACategoryTabOnlyItsOwnMail() {
        val list = listOf(msg("1", "sanne@x.nl", 3), msg("2", "news@shop.nl", 2), msg("3", "me@x.nl", 1, role = FolderRole.SENT))
        val cats = listOf(category("needs-you"), category("promotions")).associateBy { it.id }
        // Sent mail has no category, so the lookup leaves it out.
        val lookup = CategoryLookup(mapOf("1" to "needs-you", "2" to "promotions"))
        assertEquals(list, ListModel.categoryRows(list, lookup, tab = null))
        assertEquals(listOf("2"), ListModel.categoryRows(list, lookup, tab = "promotions").map { it.id })
        // Inbox, a search, VIP and Unread mix the categories: each inbox email has its label.
        assertEquals(listOf("needs-you", "promotions", null), list.map { ListModel.rowCategory(it, lookup, tab = null, cats)?.id })
        // A category's own tab says which one it is in its header, so its rows don't.
        assertNull(ListModel.rowCategory(list[1], lookup, tab = "promotions", cats))
        // A category removed meanwhile has no label to show; categories off has no lookup.
        assertNull(ListModel.rowCategory(list[0], lookup, null, cats - "needs-you"))
        assertNull(ListModel.rowCategory(list[0], CategoryLookup.NONE, null, cats))
    }

    @Test
    fun inboxLeavesOutAHiddenCategoryWhichItsTabSearchAndCountsStillKnow() {
        val list = listOf(msg("1", "sanne@x.nl", 3, unread = true), msg("2", "news@shop.nl", 2, unread = true), msg("3", "app@x.nl", 1))
        val lookup = CategoryLookup(mapOf("1" to "needs-you", "2" to "promotions", "3" to "updates"), hiddenCategories = setOf("promotions"))
        // Inbox itself: without Promotions.
        assertEquals(listOf("1", "3"), ListModel.categoryRows(list, lookup, tab = null, inbox = true).map { it.id })
        // Its own tab still shows all of it.
        assertEquals(listOf("2"), ListModel.categoryRows(list, lookup, tab = "promotions", inbox = true).map { it.id })
        // A search, VIP and Unread aren't Inbox itself: everything.
        assertEquals(list, ListModel.categoryRows(list, lookup, tab = null, inbox = false))
        // Inbox's unread count in the drawer leaves it out too.
        assertEquals(1, ListModel.inboxUnread(list.filter { it.unread }, lookup))
        assertEquals(2, ListModel.inboxUnread(list.filter { it.unread }, lookup.copy(hiddenCategories = emptySet())))
    }

    @Test
    fun pillsShrinkInProportionBeforeTheSubjectDoes() {
        // Room enough: each as wide as it needs.
        assertEquals(listOf(30, 80), ListModel.pillWidths(listOf(30, 80), 150))
        // Short of room: the longer label gives up more, so a short VIP pill stays readable longer.
        assertEquals(listOf(20, 53, 26), ListModel.pillWidths(listOf(30, 80, 40), 100))
        assertTrue(ListModel.pillWidths(listOf(30, 80, 40), 100).sum() <= 100)
        assertEquals(listOf(0, 0), ListModel.pillWidths(listOf(30, 80), 0))
    }
}
