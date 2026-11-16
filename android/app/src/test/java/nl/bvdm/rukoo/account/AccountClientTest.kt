package nl.bvdm.rukoo.account

import java.util.concurrent.atomic.AtomicLong
import kotlinx.coroutines.runBlocking
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import okhttp3.OkHttpClient
import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test

/**
 * What one answer from an account server costs: every byte read off its connection counts, as on desktop
 * (src/main/account/client.js), so HTTP framing around a small body can't go uncounted.
 */
class AccountClientTest {
    private val http = OkHttpAccountHttp(OkHttpClient())

    @Test
    fun `a long chunk extension in front of a small body ends the answer at the limit`() = runBlocking {
        // 64 MiB of chunk extension and then two bytes of JSON. OkHttp reads an extension of any length, so only
        // counting the connection stops it.
        val extension = 64L * 1024 * 1024
        RawHttpServer { _, out -> RawHttpServer.chunked(out, "{}".toByteArray(), extension) }.use { srv ->
            val meter = AtomicLong()
            try {
                http.request(srv.url, "GET", "/v1/records?since=0", null, "token", meter)
                fail("expected too_large")
            } catch (e: AccountError) {
                assertEquals("too_large", e.code)
            }
            assertTrue("the server is done", srv.awaitServed())
            assertEquals("the server couldn't send its whole answer", 1, srv.cutOff.get())
            assertTrue("the server wrote less than the answer (${srv.written.get()})", srv.written.get() < extension)
            val limit = OkHttpAccountHttp.RAW_LIMIT
            assertTrue("the meter has what was read, up to the limit (${meter.get()})", meter.get() > limit && meter.get() < limit + 1024 * 1024)
        }
    }

    @Test
    fun `a short chunk extension is read and counted`() = runBlocking {
        RawHttpServer { _, out -> RawHttpServer.chunked(out, """{"ok":true}""".toByteArray(), 100_000) }.use { srv ->
            val meter = AtomicLong()
            val res = http.request(srv.url, "GET", "/v1/info", null, null, meter) as JsonObject
            assertEquals(JsonPrimitive(true), res["ok"])
            assertTrue("the extension counts (${meter.get()})", meter.get() > 100_000)
            assertTrue(srv.awaitServed())
            assertEquals("the meter has every byte the server sent", srv.written.get(), meter.get())
        }
    }

    @Test
    fun `headers count, and each call has a connection of its own`() = runBlocking {
        val pad = "x".repeat(200 * 1024)
        val body = """{"ok":true}"""
        val srv = MockWebServer()
        srv.enqueue(MockResponse().setHeader("Content-Type", "application/json").setHeader("X-Pad", pad).setBody(body))
        srv.enqueue(MockResponse().setHeader("Content-Type", "application/json").setBody(body))
        srv.start()
        try {
            val url = "http://127.0.0.1:${srv.port}"
            val padded = AtomicLong()
            assertEquals(JsonPrimitive(true), (http.request(url, "GET", "/v1/info", null, null, padded) as JsonObject)["ok"])
            assertTrue("200 KiB of headers count (${padded.get()})", padded.get() > pad.length)
            val plain = AtomicLong()
            assertEquals(JsonPrimitive(true), (http.request(url, "GET", "/v1/info", null, null, plain) as JsonObject)["ok"])
            assertTrue("an ordinary answer counts its body and headers (${plain.get()})", plain.get() > body.length && plain.get() < 1024)
            for (i in 0 until 2) {
                val request = srv.takeRequest()
                assertEquals("each call asks for its connection to end", "close", request.getHeader("Connection"))
                assertEquals("and comes on a new one", 0, request.sequenceNumber)
            }
        } finally {
            srv.shutdown()
        }
    }

    @Test
    fun `a body over the limit is refused, and what was read counts`() = runBlocking {
        val srv = MockWebServer()
        srv.enqueue(MockResponse().setHeader("Content-Type", "application/json").setBody(okio.Buffer().write(ByteArray((OkHttpAccountHttp.MAX_BYTES + 10).toInt()))))
        srv.start()
        try {
            val meter = AtomicLong()
            try {
                http.request("http://127.0.0.1:${srv.port}", "GET", "/v1/records?since=0", null, "token", meter)
                fail("expected too_large")
            } catch (e: AccountError) {
                assertEquals("too_large", e.code)
            }
            assertTrue("the body read counts (${meter.get()})", meter.get() > OkHttpAccountHttp.MAX_BYTES)
        } finally {
            srv.shutdown()
        }
    }
}
