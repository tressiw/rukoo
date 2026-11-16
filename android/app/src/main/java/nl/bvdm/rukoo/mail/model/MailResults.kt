package nl.bvdm.rukoo.mail.model

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import nl.bvdm.rukoo.core.Change

/** Sync state per account. [errors] holds a friendly, user-facing message per failing account. */
data class SyncStatus(
    val running: Set<String> = emptySet(),
    val errors: Map<String, String> = emptyMap(),
    /** Last successful sync per account, ms. */
    val lastSync: Map<String, Long> = emptyMap(),
) {
    val syncing: Boolean get() = running.isNotEmpty()
}

/**
 * Unread and total counts, desktop's counts(scope) (engine.js:1159-1193): [accounts] is the unread
 * inbox count per account, [all] their sum, [views] unread for INBOX/UNREAD/VIP/JUNK/ARCHIVE and the
 * total for STARRED/DRAFTS/SENT/TRASH, [folders] the unread count per cached folder path (one account).
 * The inbox counts include [olderUnread], the unread inbox mail older than the newest [inboxLimit]
 * messages of each inbox, which the lists don't hold.
 */
data class MailCounts(
    val accounts: Map<String, Int> = emptyMap(),
    val all: Int = 0,
    val views: Map<ViewKind, Int> = emptyMap(),
    val folders: Map<String, Int> = emptyMap(),
    val olderUnread: Int = 0,
    val inboxLimit: Int = 0,
)

/** One moved message: its id before the move, its new id when the server reported it, and whether undo can bring it back. */
data class Moved(val oldId: String, val newId: String?, val undoable: Boolean)

data class Failed(val id: String, val error: String)

/**
 * The outcome of archive, trash or move. Undo takes the old ids, like desktop's undoMove(oldId)
 * (engine.js:1138-1165), so a toast and an agent's notice can both undo by the ids they showed.
 */
data class MoveResult(
    val moved: List<Moved> = emptyList(),
    val failed: List<Failed> = emptyList(),
    /** The folder the messages went to. */
    val folder: String? = null,
) {
    val undoIds: List<String> get() = moved.filter { it.undoable }.map { it.oldId }
}

/** The outcome of undoing moves: new ids of the restored messages, and the ones that couldn't come back. */
data class UndoResult(val restored: List<String> = emptyList(), val failed: List<Failed> = emptyList())

/** What deleting did (desktop's remove: `trash` | `confirm` | `deleted`). */
enum class DeleteOutcome {
    /** Moved to Trash; undoable. */
    TRASHED,
    /** The message is in Trash already or the account has none: deleting is permanent, so ask first and call again with force. */
    NEEDS_CONFIRM,
    /** Removed for good. */
    DELETED,
}

data class DeleteResult(val outcome: DeleteOutcome, val move: MoveResult = MoveResult())

/** How an unsubscribe went (spec 4 §3.12). */
sealed interface UnsubscribeResult {
    /** One-click POST accepted. */
    data object Done : UnsubscribeResult

    /** The sender only offers a web page: open it in the browser. Rukoo can't tell whether it worked. */
    data class OpenPage(val url: String) : UnsubscribeResult

    /** The sender wants an email: open the composer with this message for the user to send. */
    data class SendEmail(val to: String, val subject: String, val body: String) : UnsubscribeResult

    /** No List-Unsubscribe header. */
    data object Unavailable : UnsubscribeResult
}

/** Engine events beyond the state flows. */
sealed interface MailEvent {
    /** Fresh unread inbox mail found by a sync, for notifications (spec 4 §1.13). */
    data class NewMail(val messages: List<MessageSummary>) : MailEvent

    data class AccountRemoved(val accountId: String) : MailEvent
}

/** What setup sends to add an IMAP account. The engine verifies IMAP and SMTP before it saves anything. */
data class ImapAccountInput(
    val email: String,
    val name: String,
    val password: String,
    val imap: ServerConfig,
    val smtp: ServerConfig,
    val provider: String = "other",
) {
    override fun toString() = "ImapAccountInput(email=$email, provider=$provider)"
}

/** The result of Google sign-in (spec 4 §4.3): the refresh token is the account's secret. */
data class GoogleTokens(
    val email: String,
    val name: String,
    val refreshToken: String,
    val accessToken: String?,
    val expiresAt: Long?,
    val scopes: List<String>,
) {
    override fun toString() = "GoogleTokens(email=$email, scopes=$scopes)"
}

/** A Google OAuth client (Desktop type), as desktop keeps it and exports it to the phone. */
@Serializable
data class GoogleClient(
    @SerialName("client_id") val clientId: String,
    @SerialName("client_secret") val clientSecret: String = "",
) {
    override fun toString() = "GoogleClient(clientId=$clientId)"
}

/**
 * An account in desktop's export bundle (spec 3 §7.10). [password] or [refreshToken] carries the
 * secret in plain text, only between the computer and this phone; [secretError] says it couldn't be
 * decrypted over there.
 */
@Serializable
data class ImportedAccount(
    val id: String,
    val type: AccountType = AccountType.IMAP,
    val provider: String = "other",
    val auth: AuthKind = AuthKind.PASSWORD,
    val email: String,
    val name: String = "",
    val label: String = "",
    val color: String = "",
    val signature: String? = null,
    val signatureHtml: String? = null,
    val imap: ServerConfig? = null,
    val smtp: ServerConfig? = null,
    val aliases: List<Alias> = emptyList(),
    val defaultFrom: String? = null,
    val scopes: List<String> = emptyList(),
    val createdAt: Long = 0,
    val password: String? = null,
    val refreshToken: String? = null,
    val secretError: String? = null,
) {
    override fun toString() = "ImportedAccount(id=$id, email=$email, auth=$auth)"
}

/**
 * Account fields the user can change in Settings. Null leaves a field as it is; for the nullable
 * fields `Change(null)` resets them (signature back to the general one, From back to the account's address).
 */
data class AccountPatch(
    val name: String? = null,
    val label: String? = null,
    val color: String? = null,
    val signature: Change<String?>? = null,
    val signatureHtml: Change<String?>? = null,
    val defaultFrom: Change<String?>? = null,
    val aliases: List<Alias>? = null,
    /** A new password, re-verified before it is saved. */
    val password: String? = null,
) {
    override fun toString() = "AccountPatch(name=$name, label=$label, color=$color)"
}
