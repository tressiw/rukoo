package nl.bvdm.rukoo.account

import java.io.Closeable
import java.io.FilterOutputStream
import java.io.IOException
import java.io.InputStream
import java.io.OutputStream
import java.net.InetAddress
import java.net.ServerSocket
import java.net.Socket
import java.util.concurrent.LinkedBlockingQueue
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger
import java.util.concurrent.atomic.AtomicLong
import kotlin.concurrent.thread

/**
 * An HTTP/1.1 server on a plain socket, for answers MockWebServer can't give, such as chunk extensions. It reads the
 * request line and headers of each connection's one request, lets [answer] write the whole response, and closes the
 * connection. A request body isn't read: the tests send GETs here.
 */
class RawHttpServer(private val answer: (target: String, out: OutputStream) -> Unit) : Closeable {
    private val server = ServerSocket(0, 50, InetAddress.getLoopbackAddress())
    val url = "http://127.0.0.1:${server.localPort}"

    /** Connections accepted. */
    val connections = AtomicInteger()

    /** Bytes of responses written, counted as each write returns. */
    val written = AtomicLong()

    /** Responses that couldn't be written whole because the client hung up. */
    val cutOff = AtomicInteger()

    private val served = LinkedBlockingQueue<Unit>()

    init {
        thread(isDaemon = true, name = "raw-http") {
            while (true) {
                val client = try {
                    server.accept()
                } catch (e: IOException) {
                    break
                }
                connections.incrementAndGet()
                thread(isDaemon = true, name = "raw-http-connection") { serve(client) }
            }
        }
    }

    /** Waits for the next connection to be done with, so what it wrote is in [written] and [cutOff]. */
    fun awaitServed(timeoutSeconds: Long = 30): Boolean = served.poll(timeoutSeconds, TimeUnit.SECONDS) != null

    private fun serve(client: Socket) {
        try {
            client.use {
                val target = requestTarget(it.getInputStream()) ?: return
                val out = Counted(it.getOutputStream().buffered(64 * 1024))
                try {
                    answer(target, out)
                    out.flush()
                } catch (e: IOException) {
                    cutOff.incrementAndGet()
                }
            }
        } finally {
            served.put(Unit)
        }
    }

    /** The request's target, such as `/v1/records?since=0`, once its headers are in; null when the client went away. */
    private fun requestTarget(input: InputStream): String? {
        val line = StringBuilder()
        var first: String? = null
        while (true) {
            val b = input.read()
            if (b < 0) return null
            if (b == '\n'.code) {
                val text = line.toString().trimEnd('\r')
                line.setLength(0)
                if (first == null) first = text else if (text.isEmpty()) return first.split(' ').getOrNull(1)
            } else {
                line.append(b.toChar())
            }
        }
    }

    private inner class Counted(out: OutputStream) : FilterOutputStream(out) {
        override fun write(b: ByteArray, off: Int, len: Int) {
            out.write(b, off, len)
            written.addAndGet(len.toLong())
        }

        override fun write(b: Int) {
            out.write(b)
            written.incrementAndGet()
        }
    }

    override fun close() {
        server.close()
    }

    companion object {
        /**
         * A 200 answer of [body] as one chunk, behind a chunk extension of [extension] bytes: `;` and that many `x`s
         * minus one, written in pieces so a long one never sits in memory. Returns the bytes of the whole answer.
         */
        fun chunked(out: OutputStream, body: ByteArray, extension: Long): Long {
            val head = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n".toByteArray()
            val size = body.size.toString(16).toByteArray()
            val end = "\r\n0\r\n\r\n".toByteArray()
            out.write(head)
            out.write(size)
            if (extension > 0) {
                out.write(';'.code)
                val piece = ByteArray(64 * 1024) { 'x'.code.toByte() }
                var left = extension - 1
                while (left > 0) {
                    val n = minOf(left, piece.size.toLong()).toInt()
                    out.write(piece, 0, n)
                    left -= n
                }
            }
            out.write("\r\n".toByteArray())
            out.write(body)
            out.write(end)
            return head.size + size.size + extension + 2 + body.size + end.size
        }
    }
}
