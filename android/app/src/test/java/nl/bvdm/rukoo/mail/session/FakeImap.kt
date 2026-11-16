package nl.bvdm.rukoo.mail.session

import java.io.BufferedReader
import java.io.InputStreamReader
import java.io.OutputStream
import java.net.ServerSocket
import java.net.Socket
import java.util.concurrent.CopyOnWriteArrayList
import kotlin.concurrent.thread

/**
 * A small scripted IMAP server on a local socket, for ImapSession's syncs: one INBOX with [messages], which answers the
 * fetches a sync makes and records every command it got.
 */
internal class FakeImap(messages: List<Msg>, var capabilities: String, val tabs: Map<String, List<Long>> = emptyMap()) : AutoCloseable {
    /** One message: its uid, flags, the header lines a header fetch returns, its BODYSTRUCTURE and the text of its body. */
    class Msg(val uid: Long, val subject: String, val headers: String = "", val flags: String = "", val structure: String = TEXT_PLAIN, val text: String? = null)

    val messages = CopyOnWriteArrayList(messages)
    val commands = CopyOnWriteArrayList<String>()

    /** Answers a search for unread mail with NO, as a server that turns it down. */
    @Volatile
    var refuseSearch = false
    private val server = ServerSocket(0, 50, java.net.InetAddress.getByName("127.0.0.1"))
    val port: Int get() = server.localPort

    init {
        thread(isDaemon = true) {
            while (!server.isClosed) {
                val socket = runCatching { server.accept() }.getOrNull() ?: break
                // use closes the socket however serve ends: LOGOUT, the client hanging up, or a failed write.
                thread(isDaemon = true) { socket.use { runCatching { serve(it) } } }
            }
        }
    }

    override fun close() = server.close()

    private fun expand(set: String): List<Long> = set.split(',').flatMap { range ->
        val a = range.substringBefore(':').toLong()
        val b = range.substringAfter(':', range).toLong()
        (a..b).toList()
    }

    private fun serve(socket: Socket) {
        val input = BufferedReader(InputStreamReader(socket.getInputStream(), Charsets.ISO_8859_1))
        val out: OutputStream = socket.getOutputStream()
        fun send(text: String) {
            out.write(text.toByteArray(Charsets.ISO_8859_1))
            out.flush()
        }
        send("* OK [CAPABILITY $capabilities] ready\r\n")
        while (true) {
            val line = input.readLine() ?: break
            val tag = line.substringBefore(' ')
            val command = line.substringAfter(' ')
            val verb = command.substringBefore(' ').uppercase()
            commands += command
            when {
                verb == "CAPABILITY" -> send("* CAPABILITY $capabilities\r\n$tag OK done\r\n")
                verb == "LOGIN" -> send("$tag OK [CAPABILITY $capabilities] logged in\r\n")
                verb == "LOGOUT" -> {
                    send("* BYE bye\r\n$tag OK done\r\n")
                    socket.close()
                    return
                }
                verb == "LIST" -> send("* LIST (\\HasNoChildren) \"/\" INBOX\r\n$tag OK done\r\n")
                verb == "EXAMINE" || verb == "SELECT" -> send(
                    "* ${messages.size} EXISTS\r\n* 0 RECENT\r\n* FLAGS (\\Seen \\Flagged \\Answered)\r\n* OK [UIDVALIDITY 9] ok\r\n" +
                        "* OK [UIDNEXT ${messages.maxOf { it.uid } + 1}] ok\r\n$tag OK [${if (verb == "EXAMINE") "READ-ONLY" else "READ-WRITE"}] done\r\n",
                )
                verb == "FETCH" -> {
                    // FETCH <start>:* answers for the messages from that sequence number on.
                    val start = Regex("^FETCH (\\d+):").find(command)?.groupValues?.get(1)?.toInt() ?: 1
                    val sb = StringBuilder()
                    messages.forEachIndexed { i, m -> if (i + 1 >= start) sb.append("* ${i + 1} FETCH (UID ${m.uid} FLAGS (${m.flags}))\r\n") }
                    send("$sb$tag OK done\r\n")
                }
                command.startsWith("UID FETCH") -> {
                    val set = command.split(' ')[2]
                    val sb = StringBuilder()
                    for (uid in expand(set)) {
                        val i = messages.indexOfFirst { it.uid == uid }
                        if (i < 0) continue
                        val m = messages[i]
                        val structure = if (command.contains("BODYSTRUCTURE")) " BODYSTRUCTURE ${m.structure}" else ""
                        when {
                            command.contains("HEADER.FIELDS") -> {
                                val section = command.substringAfter("BODY.PEEK[").substringBefore("]")
                                // A partial fetch (<0.n>) gets the first n bytes, as a server sends them.
                                val most = Regex("\\]<0\\.(\\d+)>").find(command)?.groupValues?.get(1)?.toInt()
                                val body = (m.headers + "\r\n").let { if (most != null) it.take(most) else it }
                                val origin = if (most != null) "<0>" else ""
                                sb.append("* ${i + 1} FETCH (UID ${m.uid} BODY[$section]$origin {${body.length}}\r\n$body)\r\n")
                            }
                            command.contains("ENVELOPE") -> sb.append(
                                "* ${i + 1} FETCH (UID ${m.uid} RFC822.SIZE 105 INTERNALDATE \"01-Jan-2026 10:00:00 +0000\" " +
                                    "ENVELOPE (\"Thu, 1 Jan 2026 10:00:00 +0000\" \"${m.subject}\" ((\"Sender\" NIL \"s\" \"example.com\")) " +
                                    "((\"Sender\" NIL \"s\" \"example.com\")) NIL ((NIL NIL \"me\" \"example.com\")) NIL NIL NIL \"<m${m.uid}@example.com>\")" +
                                    "$structure)\r\n",
                            )
                            structure.isNotEmpty() -> sb.append("* ${i + 1} FETCH (UID ${m.uid}$structure)\r\n")
                            // A preview: the first bytes of the part, for a message that has text.
                            command.contains("BODY.PEEK[") && m.text != null -> {
                                val section = command.substringAfter("BODY.PEEK[").substringBefore("]")
                                sb.append("* ${i + 1} FETCH (UID ${m.uid} BODY[$section]<0> {${m.text.length}}\r\n${m.text})\r\n")
                            }
                            else -> Unit
                        }
                    }
                    send("$sb$tag OK done\r\n")
                }
                // Unread mail in a uid range, as a count with ESEARCH's RETURN (COUNT) or else as uids.
                command.startsWith("UID SEARCH") && command.endsWith(" UNSEEN") -> {
                    val range = Regex("UID (\\d+):(\\d+) UNSEEN").find(command)!!.groupValues
                    val uids = messages.filter { it.uid in range[1].toLong()..range[2].toLong() && "\\Seen" !in it.flags }.map { it.uid }
                    when {
                        refuseSearch -> send("$tag NO search not allowed\r\n")
                        command.contains("RETURN (COUNT)") -> send("* ESEARCH (TAG \"$tag\") UID COUNT ${uids.size}\r\n$tag OK done\r\n")
                        else -> send("* SEARCH${uids.joinToString("") { " $it" }}\r\n$tag OK done\r\n")
                    }
                }
                command.startsWith("UID SEARCH") -> {
                    val tab = Regex("category:(\\w+)").find(command)?.groupValues?.get(1)
                    val from = Regex("UID (\\d+):\\*").find(command)?.groupValues?.get(1)?.toLong() ?: 0
                    val uids = tabs[tab].orEmpty().filter { it >= from }
                    send("* SEARCH${uids.joinToString("") { " $it" }}\r\n$tag OK done\r\n")
                }
                else -> send("$tag OK done\r\n")
            }
        }
    }

    companion object {
        /** The structure of a short plain text message. */
        const val TEXT_PLAIN = "(\"text\" \"plain\" (\"charset\" \"utf-8\") NIL NIL \"7bit\" 5 1 NIL NIL NIL NIL)"
    }
}
