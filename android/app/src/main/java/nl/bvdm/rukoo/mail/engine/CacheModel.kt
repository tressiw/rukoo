package nl.bvdm.rukoo.mail.engine

import kotlinx.serialization.Serializable
import nl.bvdm.rukoo.mail.model.Account
import nl.bvdm.rukoo.mail.model.CachedMessage
import nl.bvdm.rukoo.mail.model.FolderRole
import nl.bvdm.rukoo.mail.model.MailPrefs
import nl.bvdm.rukoo.mail.session.FolderRecord

/**
 * The newest window of one folder (spec 4 §1.5): UIDVALIDITY, EXISTS, when it synced, and its messages. An inbox also
 * keeps [olderUnseen], its unread mail older than [messages] as the server counted it at that sync; null when it
 * couldn't (MailViews.olderUnread).
 */
@Serializable
data class CachedBox(
    val uidValidity: String,
    val exists: Int = 0,
    val syncedAt: Long = 0,
    val messages: List<CachedMessage> = emptyList(),
    val olderUnseen: Int? = null,
)

/**
 * Where Rukoo moved a message, by its Message-ID key (spec 4 §1.11): the folder it is in now and the
 * last few folders it came through.
 */
@Serializable
data class MoveNote(val id: String, val folder: String, val past: List<String> = emptyList())

/** One account's cache file, `cache/<accountId>.json` (spec 4 §1.5). */
@Serializable
data class AccountCache(
    val folders: List<FolderRecord> = emptyList(),
    val boxes: Map<String, CachedBox> = emptyMap(),
    val lastSync: Long? = null,
    val error: String? = null,
    val moves: List<MoveNote> = emptyList(),
) {
    fun folder(path: String): FolderRecord? = folders.firstOrNull { it.path == path }

    fun folderByRole(role: FolderRole): FolderRecord? = folders.firstOrNull { it.role == role }

    /** Gmail has no archive folder; archiving there means moving to All Mail. */
    fun archiveFolder(): FolderRecord? = folderByRole(FolderRole.ARCHIVE) ?: folderByRole(FolderRole.ALL)
}

/** Everything the lists and counts are computed from, as one consistent snapshot. */
data class MailSnapshot(
    val accounts: List<Account> = emptyList(),
    val caches: Map<String, AccountCache> = emptyMap(),
    val prefs: MailPrefs = MailPrefs(),
)

/** Engine limits from desktop (engine.js:18-33, spec 4 §1.12). */
object Limits {
    const val INBOX = 300
    const val DEFAULT = 100
    const val PREFETCH_COUNT = 30
    const val PREFETCH_MAX_BYTES = 1024 * 1024L
    const val UNDO_MS = 10 * 60 * 1000L
    const val MOVES_KEPT = 500
    const val MOVE_PAST = 5
    const val OPEN_FOLDER_FRESH_MS = 60_000L
    const val CACHE_SAVE_DELAY_MS = 250L

    /** Folders every sync covers. */
    val SYNC_ROLES = setOf(FolderRole.INBOX, FolderRole.SENT, FolderRole.DRAFTS, FolderRole.TRASH, FolderRole.JUNK, FolderRole.ARCHIVE)

    /** Folders whose mail is not archived, even though Gmail's All Mail also holds it. */
    val SYSTEM_ROLES = setOf(FolderRole.INBOX, FolderRole.SENT, FolderRole.DRAFTS, FolderRole.TRASH, FolderRole.JUNK)
}
