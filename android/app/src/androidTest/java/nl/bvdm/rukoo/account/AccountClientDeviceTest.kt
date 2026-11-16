package nl.bvdm.rukoo.account

import java.io.Closeable
import java.io.FilterOutputStream
import java.io.IOException
import java.io.InputStream
import java.io.OutputStream
import java.util.concurrent.CopyOnWriteArrayList
import java.util.concurrent.LinkedBlockingQueue
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicLong
import javax.net.ssl.ExtendedSSLSession
import javax.net.ssl.SNIHostName
import javax.net.ssl.SSLSocket
import kotlin.concurrent.thread
import kotlinx.coroutines.runBlocking
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import okhttp3.OkHttpClient
import okhttp3.tls.HandshakeCertificates
import okhttp3.tls.HeldCertificate
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test

/**
 * The account client over the phone's own TLS (Conscrypt), which a JVM test doesn't have: the counting socket only
 * works when the TLS socket reads the connection through it. Both ends run on the device, over localhost.
 */
class AccountClientDeviceTest {
    private val held = HeldCertificate.Builder().addSubjectAlternativeName("localhost").build()
    private val serverCerts = HandshakeCertificates.Builder().heldCertificate(held).build()
    private val clientCerts = HandshakeCertificates.Builder().addTrustedCertificate(held.certificate).build()
    private val http = OkHttpAccountHttp(OkHttpClient.Builder().sslSocketFactory(clientCerts.sslSocketFactory(), clientCerts.trustManager).build())
    private val servers = mutableListOf<TlsServer>()

    @After
    fun tearDown() {
        servers.forEach { it.close() }
    }

    @Test
    fun headersAndChunkFramingCountOverTls() = runBlocking {
        val pad = "x".repeat(200 * 1024)
        val srv = server { out -> chunked(out, """{"ok":true}""".toByteArray(), 100_000, "X-Pad: $pad\r\n") }
        val meter = AtomicLong()
        val res = http.request(srv.url, "GET", "/v1/info", null, null, meter) as JsonObject
        assertEquals(JsonPrimitive(true), res["ok"])
        assertTrue(srv.awaitServed())
        val written = srv.written.get()
        // Read encrypted, the answer takes a little more than the server wrote: the handshake and a few bytes per record.
        assertTrue("the meter has at least what the server wrote ($written): ${meter.get()}", meter.get() >= written)
        assertTrue("and not much more: ${meter.get()}", meter.get() < written + 64 * 1024)
        assertEquals("the phone named the server it wanted", listOf("localhost"), srv.serverNames.toList())
    }

    @Test
    fun aLongChunkExtensionOverTlsEndsTheAnswerAtTheLimit() = runBlocking {
        val extension = 64L * 1024 * 1024
        val srv = server { out -> chunked(out, "{}".toByteArray(), extension) }
        val meter = AtomicLong()
        try {
            http.request(srv.url, "GET", "/v1/records?since=0", null, "token", meter)
            fail("expected too_large")
        } catch (e: AccountError) {
            assertEquals("too_large", e.code)
        }
        assertTrue(srv.awaitServed())
        assertTrue("the server couldn't send its whole answer", srv.cutOff.get())
        assertTrue("the server wrote less than the answer (${srv.written.get()})", srv.written.get() < extension)
        val limit = OkHttpAccountHttp.RAW_LIMIT
        assertTrue("the meter has what was read, up to the limit (${meter.get()})", meter.get() > limit && meter.get() < limit + 1024 * 1024)
    }

    private fun server(answer: (OutputStream) -> Unit) = TlsServer(serverCerts, answer).also { servers += it }

    /** A 200 answer of [body] as one chunk, behind a chunk extension of [extension] bytes, written in pieces. */
    private fun chunked(out: OutputStream, body: ByteArray, extension: Long, headers: String = "") {
        out.write("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n$headers\r\n".toByteArray())
        out.write(body.size.toString(16).toByteArray())
        out.write(';'.code)
        val piece = ByteArray(64 * 1024) { 'x'.code.toByte() }
        var left = extension - 1
        while (left > 0) {
            val n = minOf(left, piece.size.toLong()).toInt()
            out.write(piece, 0, n)
            left -= n
        }
        out.write("\r\n".toByteArray())
        out.write(body)
        out.write("\r\n0\r\n\r\n".toByteArray())
    }

    /** An HTTPS server on the device: one request per connection, answered by [answer], then the connection ends. */
    private class TlsServer(certs: HandshakeCertificates, private val answer: (OutputStream) -> Unit) : Closeable {
        private val server = certs.sslContext().serverSocketFactory.createServerSocket(0)
        val url = "https://localhost:${server.localPort}"
        val written = AtomicLong()
        val cutOff = AtomicBoolean()
        val serverNames = CopyOnWriteArrayList<String>()
        private val served = LinkedBlockingQueue<Unit>()

        init {
            thread(isDaemon = true, name = "tls-server") {
                while (true) {
                    val socket = try {
                        server.accept() as SSLSocket
                    } catch (e: IOException) {
                        break
                    }
                    thread(isDaemon = true, name = "tls-connection") { serve(socket) }
                }
            }
        }

        fun awaitServed(): Boolean = served.poll(60, TimeUnit.SECONDS) != null

        private fun serve(socket: SSLSocket) {
            try {
                socket.use {
                    it.startHandshake()
                    (it.session as? ExtendedSSLSession)?.requestedServerNames?.forEach { name -> if (name is SNIHostName) serverNames += name.asciiName }
                    if (!readHeaders(it.inputStream)) return
                    val counted = object : FilterOutputStream(it.outputStream.buffered(64 * 1024)) {
                        override fun write(b: ByteArray, off: Int, len: Int) {
                            out.write(b, off, len)
                            written.addAndGet(len.toLong())
                        }

                        override fun write(b: Int) {
                            out.write(b)
                            written.incrementAndGet()
                        }
                    }
                    try {
                        answer(counted)
                        counted.flush()
                    } catch (e: IOException) {
                        cutOff.set(true)
                    }
                }
            } catch (e: IOException) {
                // The handshake or the request failed; the test sees that on its side.
            } finally {
                served.put(Unit)
            }
        }

        /** Reads up to the end of the request's headers; false when the client went away first. */
        private fun readHeaders(input: InputStream): Boolean {
            var tail = 0
            while (true) {
                val b = input.read()
                if (b < 0) return false
                tail = (tail shl 8) or b
                if (tail == 0x0d0a0d0a) return true
            }
        }

        override fun close() {
            server.close()
        }
    }
}
