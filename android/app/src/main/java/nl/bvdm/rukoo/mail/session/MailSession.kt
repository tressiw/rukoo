package nl.bvdm.rukoo.mail.session

import kotlinx.serialization.Serializable
import nl.bvdm.rukoo.mail.mime.ComposedMail
import nl.bvdm.rukoo.mail.model.CachedMessage
import nl.bvdm.rukoo.mail.model.FolderRole

/** A folder as the server lists it, with its role (spec 4 §1.4). Kept in the account's cache. */
@Serializable
data class FolderRecord(
    val path: String,
    val name: String,
    val delimiter: String? = "/",
    val specialUse: String? = null,
    val role: FolderRole = FolderRole.NONE,
)

/**
 * The newest window of a folder: its UIDVALIDITY, EXISTS and messages in sequence order. [olderUnseen]: how many
 * messages older than the window are unread, when the sync was asked for it and the server said; else null.
 */
data class FolderSync(val uidValidity: String, val exists: Int, val messages: List<CachedMessage>, val olderUnseen: Int? = null)

/** Where moved messages landed (old uid → new uid), as far as the server says (UIDPLUS); empty and null without it. */
data class MoveOutcome(val uids: Map<Long, Long>, val uidValidity: String?)

/** Thrown by [MailSession.move] when the source folder's UIDVALIDITY isn't the expected one. */
class UidValidityChanged : Exception("UIDVALIDITY changed")

/**
 * One account's connection to its server: IMAP and SMTP, or the demo account's simulated server
 * (spec 4 §0). Methods throw on failure; the engine turns errors into text for the user.
 */
interface MailSession {
    suspend fun verify()

    suspend fun listFolders(background: Boolean = false): List<FolderRecord>

    /** Creates a top-level folder in the personal namespace; returns its path. */
    suspend fun createFolder(name: String): String

    /**
     * The newest [limit] messages of [path] by sequence number. Messages in [known] (cached under the
     * same [uidValidity]) only get their flags refreshed; the rest are fetched in full. [signals]: also what kind of
     * mail each message is (MailSignals), for new ones and cached ones without; [gmailTabs]: also the Gmail tab each
     * message is in. Both are for the inbox and its categories. [olderUnseen]: also how many messages older than the
     * window are unread, which the inbox's unread count adds to the window's own (FolderSync.olderUnseen).
     */
    suspend fun syncFolder(
        path: String,
        limit: Int,
        known: Map<Long, CachedMessage>,
        uidValidity: String?,
        background: Boolean = false,
        signals: Boolean = false,
        gmailTabs: Boolean = false,
        olderUnseen: Boolean = false,
    ): FolderSync

    /** The raw source of a message. Doesn't mark it read. */
    suspend fun fetchSource(path: String, uid: Long, background: Boolean = false): ByteArray

    /** Changes flags of [uids] in [path] in one command per flag (desktop sends one per message). */
    suspend fun setFlags(path: String, uids: List<Long>, unread: Boolean? = null, starred: Boolean? = null, answered: Boolean? = null)

    /** Moves [uids] from [path]; throws [UidValidityChanged] when [expectUidValidity] is set and the folder's differs. */
    suspend fun move(path: String, uids: List<Long>, destination: String, expectUidValidity: String? = null): MoveOutcome

    suspend fun deleteForever(path: String, uids: List<Long>)

    /** Stores [raw] in [path]; returns its uid when the server reports it (UIDPLUS). */
    suspend fun append(path: String, raw: ByteArray, flags: List<String> = emptyList()): Long?

    /** Sends [mail] and returns the raw source that went out. */
    suspend fun send(mail: ComposedMail): ByteArray

    /** True when the server files sent mail in Sent itself, so Rukoo doesn't add a copy. */
    fun savesSentCopy(): Boolean

    suspend fun close()
}

/** How a session signs in: a password, or an OAuth access token fetched per connection. */
sealed interface Credentials {
    class Password(val password: String) : Credentials {
        override fun toString() = "Password(...)"
    }

    class OAuth(val accessToken: suspend () -> String) : Credentials {
        override fun toString() = "OAuth(...)"
    }
}
