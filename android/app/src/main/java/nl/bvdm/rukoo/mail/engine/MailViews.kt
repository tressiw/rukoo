package nl.bvdm.rukoo.mail.engine

import java.text.Collator
import java.util.Locale
import nl.bvdm.rukoo.mail.model.Account
import nl.bvdm.rukoo.mail.model.Address
import nl.bvdm.rukoo.mail.model.CachedMessage
import nl.bvdm.rukoo.mail.model.FolderRole
import nl.bvdm.rukoo.mail.model.Identity
import nl.bvdm.rukoo.mail.model.MailCounts
import nl.bvdm.rukoo.mail.model.MailPrefs
import nl.bvdm.rukoo.mail.model.MailView
import nl.bvdm.rukoo.mail.model.MessageIds
import nl.bvdm.rukoo.mail.model.MessageSummary
import nl.bvdm.rukoo.mail.model.SortOrder
import nl.bvdm.rukoo.mail.model.ViewKind
import nl.bvdm.rukoo.mail.session.FolderRecord

/**
 * Lists, counts, search and contacts computed from the cache, ported from desktop's engine
 * (engine.js:775-893, 1344-1369; spec 4 §1.14-1.15, §1.18). Pure functions over a [MailSnapshot].
 */
object MailViews {
    private fun lowerAddr(a: Address?): String = a?.address.orEmpty().trim().lowercase()

    fun isSpam(prefs: MailPrefs, from: Address?) = lowerAddr(from).let { it.isNotEmpty() && it in prefs.spam }

    fun isVip(prefs: MailPrefs, from: Address?) = lowerAddr(from).let { it.isNotEmpty() && it in prefs.vips }

    fun publicMessage(acc: Account, folderPath: String, info: FolderRecord?, m: CachedMessage, prefs: MailPrefs) = MessageSummary(
        id = MessageIds.encode(acc.id, folderPath, m.uid),
        accountId = acc.id,
        accountColor = acc.color,
        folder = folderPath,
        role = info?.role ?: FolderRole.NONE,
        uid = m.uid,
        messageId = m.messageId,
        inReplyTo = m.inReplyTo,
        subject = m.subject,
        from = m.from,
        to = m.to,
        cc = m.cc,
        replyTo = m.replyTo,
        date = m.date,
        preview = m.preview,
        unread = m.unread,
        starred = m.starred,
        answered = m.answered,
        hasAttachments = m.hasAttachments,
        vip = isVip(prefs, m.from),
        signals = m.signals,
        gmailTab = m.gmailTab,
    )

    /** A view's test on (folder, message), and whether it is the archive view (for All Mail dedup). */
    class Predicate(val archive: Boolean = false, val test: (FolderRecord, CachedMessage) -> Boolean)

    fun predicate(view: MailView, prefs: MailPrefs): Predicate {
        val inbox = { info: FolderRecord, m: CachedMessage -> info.role == FolderRole.INBOX && !isSpam(prefs, m.from) }
        return when (view.kind) {
            ViewKind.INBOX -> Predicate(test = inbox)
            ViewKind.UNREAD -> Predicate { info, m -> inbox(info, m) && m.unread }
            ViewKind.VIP -> Predicate { info, m -> inbox(info, m) && isVip(prefs, m.from) }
            ViewKind.STARRED -> Predicate { info, m -> m.starred && info.role != FolderRole.TRASH && info.role != FolderRole.JUNK }
            ViewKind.FOLDER -> Predicate { info, _ -> info.path == view.folder }
            ViewKind.EVERYTHING -> Predicate { _, _ -> true }
            ViewKind.ARCHIVE -> Predicate(archive = true) { info, _ -> info.role == FolderRole.ARCHIVE || info.role == FolderRole.ALL }
            ViewKind.DRAFTS -> Predicate { info, _ -> info.role == FolderRole.DRAFTS }
            ViewKind.SENT -> Predicate { info, _ -> info.role == FolderRole.SENT }
            ViewKind.TRASH -> Predicate { info, _ -> info.role == FolderRole.TRASH }
            ViewKind.JUNK -> Predicate { info, _ -> info.role == FolderRole.JUNK }
        }
    }

    /**
     * Gmail's All Mail also holds every message of the other folders. Normally its copies of those are
     * hidden; the archive view only drops what is still in a system folder, so archived mail with a
     * label stays visible there. Compared on the raw Message-ID, like desktop.
     */
    private fun messageIdsOutside(cache: AccountCache, folderPath: String, roles: Set<FolderRole>?): Set<String> {
        val ids = HashSet<String>()
        for ((p, box) in cache.boxes) {
            if (p == folderPath) continue
            if (roles != null && cache.folder(p)?.role !in roles) continue
            for (m in box.messages) m.messageId?.let { ids += it }
        }
        return ids
    }

    fun collect(s: MailSnapshot, accountId: String?, predicate: Predicate): List<MessageSummary> {
        val out = ArrayList<MessageSummary>()
        for (acc in s.accounts) {
            if (accountId != null && acc.id != accountId) continue
            val cache = s.caches[acc.id] ?: continue
            for ((folderPath, box) in cache.boxes) {
                val info = cache.folder(folderPath) ?: FolderRecord(folderPath, folderPath)
                val elsewhere = if (info.role == FolderRole.ALL) {
                    messageIdsOutside(cache, folderPath, if (predicate.archive) Limits.SYSTEM_ROLES else null)
                } else {
                    null
                }
                for (m in box.messages) {
                    if (elsewhere != null && m.messageId != null && m.messageId in elsewhere) continue
                    if (predicate.test(info, m)) out += publicMessage(acc, folderPath, info, m, s.prefs)
                }
            }
        }
        return out
    }

    /** Desktop's search: subject, preview, sender and every To, not Cc; trimmed and lower-cased. */
    fun matches(m: MessageSummary, query: String): Boolean {
        val q = query.trim().lowercase()
        if (q.isEmpty()) return true
        val hay = buildString {
            append(m.subject).append(' ').append(m.preview).append(' ').append(m.from.name).append(' ').append(m.from.address)
            for (t in m.to) append(' ').append(t.name).append(' ').append(t.address)
        }.lowercase()
        return q in hay
    }

    fun sort(list: List<MessageSummary>, order: SortOrder, locale: Locale = Locale.getDefault()): List<MessageSummary> {
        val byDate = compareByDescending<MessageSummary> { it.date }
        return when (order) {
            SortOrder.DATE_DESC -> list.sortedWith(byDate)
            SortOrder.DATE_ASC -> list.sortedBy { it.date }
            SortOrder.UNREAD -> list.sortedWith(compareByDescending<MessageSummary> { it.unread }.then(byDate))
            SortOrder.SENDER -> {
                val collator = Collator.getInstance(locale)
                list.sortedWith(Comparator<MessageSummary> { a, b -> collator.compare(a.from.name.ifEmpty { a.from.address }, b.from.name.ifEmpty { b.from.address }) }.then(byDate))
            }
        }
    }

    fun list(s: MailSnapshot, view: MailView, query: String, order: SortOrder, locale: Locale = Locale.getDefault()): List<MessageSummary> {
        val all = collect(s, view.accountId, predicate(view, s.prefs))
        return sort(if (query.isBlank()) all else all.filter { matches(it, query) }, order, locale)
    }

    /**
     * Unread inbox mail older than the newest messages the cache keeps (Limits.INBOX), as the server counted it at the
     * last sync (engine.js olderUnread); [folderPath]: of that folder only. Mail from senders on the spam list is in it
     * too: it can't be told apart without its headers. Reading a cached message counts at once; an older one counts
     * from the next sync.
     */
    fun olderUnread(s: MailSnapshot, accountId: String?, folderPath: String? = null): Int {
        var n = 0
        for (acc in s.accounts) {
            if (accountId != null && acc.id != accountId) continue
            val cache = s.caches[acc.id] ?: continue
            for (f in cache.folders) {
                if (f.role != FolderRole.INBOX || (folderPath != null && f.path != folderPath)) continue
                n += cache.boxes[f.path]?.olderUnseen ?: 0
            }
        }
        return n
    }

    /** Unread mail in the inboxes of all accounts, older mail included: the badge. */
    fun unreadCount(s: MailSnapshot): Int = collect(s, null, predicate(MailView(ViewKind.UNREAD), s.prefs)).size + olderUnread(s, null)

    fun counts(s: MailSnapshot, accountId: String?): MailCounts {
        val perAccount = s.accounts.associate { it.id to collect(s, it.id, predicate(MailView(ViewKind.UNREAD), s.prefs)).size + olderUnread(s, it.id) }
        fun count(kind: ViewKind, onlyUnread: Boolean) =
            collect(s, accountId, predicate(MailView(kind, accountId), s.prefs)).count { !onlyUnread || it.unread }
        // The list holds only the newest Limits.INBOX messages of each inbox, so its header names the older unread mail
        // apart. Only Inbox and Unread add it: VIP picks its mail by sender, and other folders count their cache.
        val older = olderUnread(s, accountId)
        val views = mapOf(
            ViewKind.INBOX to count(ViewKind.INBOX, true) + older,
            ViewKind.UNREAD to count(ViewKind.UNREAD, true) + older,
            ViewKind.VIP to count(ViewKind.VIP, true),
            ViewKind.STARRED to count(ViewKind.STARRED, false),
            ViewKind.DRAFTS to count(ViewKind.DRAFTS, false),
            ViewKind.SENT to count(ViewKind.SENT, false),
            ViewKind.TRASH to count(ViewKind.TRASH, false),
            ViewKind.JUNK to count(ViewKind.JUNK, true),
            ViewKind.ARCHIVE to count(ViewKind.ARCHIVE, true),
        )
        val folders = if (accountId == null) {
            emptyMap()
        } else {
            s.caches[accountId]?.boxes.orEmpty().mapValues { (path, box) -> box.messages.count { it.unread } + olderUnread(s, accountId, path) }
        }
        return MailCounts(perAccount, perAccount.values.sum(), views, folders, olderUnread = older, inboxLimit = Limits.INBOX)
    }

    /** Every address an account can send from: its own first, then its aliases (engine.js:329-335). */
    fun identities(acc: Account): List<Identity> {
        val own = Identity(acc.email, acc.name, primary = true)
        val aliases = acc.aliases
            .filter { it.address.isNotEmpty() && !it.address.equals(acc.email, ignoreCase = true) }
            .map { Identity(it.address, it.name.ifEmpty { acc.name }, primary = false) }
        return listOf(own) + aliases
    }

    /** The identity for [address], or the account's default From when it is null (engine.js:337-341). */
    fun identity(acc: Account, address: String?): Identity? {
        val list = identities(acc)
        if (address.isNullOrEmpty()) return list.firstOrNull { it.address.equals(acc.defaultFrom.orEmpty(), ignoreCase = true) } ?: list.first()
        return list.firstOrNull { it.address.equals(address, ignoreCase = true) }
    }

    /**
     * Recipient suggestions: senders count once, people you wrote to (Sent) three times; no-reply
     * addresses and your own identities are left out (engine.js:1344-1369).
     */
    fun contacts(s: MailSnapshot): List<Address> {
        class Entry(var name: String, val address: String, var score: Int)
        val map = LinkedHashMap<String, Entry>()
        fun add(a: Address, weight: Int) {
            val key = lowerAddr(a)
            if (key.isEmpty() || '@' !in key || Regex("no-?reply|do-?not-?reply").containsMatchIn(key)) return
            val prev = map.getOrPut(key) { Entry("", a.address, 0) }
            if (prev.name.isEmpty() && a.name.isNotEmpty()) prev.name = a.name
            prev.score += weight
        }
        val own = s.accounts.flatMap { identities(it).map { i -> i.address.lowercase() } }.toSet()
        for (cache in s.caches.values) {
            for ((folderPath, box) in cache.boxes) {
                val sent = cache.folder(folderPath)?.role == FolderRole.SENT
                for (m in box.messages) {
                    if (sent) (m.to + m.cc).forEach { add(it, 3) } else add(m.from, 1)
                }
            }
        }
        return map.values.filter { it.address.lowercase() !in own }.sortedByDescending { it.score }.map { Address(it.name, it.address) }
    }

    /** Every cached copy with this Message-ID, live folders before Gmail's All Mail. */
    fun findByMessageId(s: MailSnapshot, messageId: String, accountId: String?): List<MessageSummary> {
        val key = MessageIds.headerKey(messageId)
        if (key.isEmpty()) return emptyList()
        val hits = ArrayList<MessageSummary>()
        for (acc in s.accounts) {
            if (accountId != null && acc.id != accountId) continue
            val cache = s.caches[acc.id] ?: continue
            for ((folderPath, box) in cache.boxes) {
                val info = cache.folder(folderPath)
                for (m in box.messages) {
                    if (MessageIds.headerKey(m.messageId) == key) hits += publicMessage(acc, folderPath, info, m, s.prefs)
                }
            }
        }
        return hits.sortedBy { if (it.role == FolderRole.ALL) 1 else 0 }
    }
}
