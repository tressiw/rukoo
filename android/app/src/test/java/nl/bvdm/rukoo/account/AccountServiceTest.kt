package nl.bvdm.rukoo.account

import java.io.File
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicInteger
import java.util.concurrent.atomic.AtomicLong
import java.util.concurrent.atomic.AtomicReference
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.async
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.addJsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import kotlinx.serialization.json.putJsonArray
import nl.bvdm.rukoo.core.Clock
import nl.bvdm.rukoo.core.SecretCipher
import okhttp3.OkHttpClient
import okhttp3.mockwebserver.Dispatcher
import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import okhttp3.mockwebserver.RecordedRequest
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder

/** Plain text that says it is a secret, so tests can see that secrets go through the Keystore box. */
object PlainSecrets : SecretCipher {
    override fun encrypt(plain: String) = "plain:" + java.util.Base64.getEncoder().encodeToString(plain.toByteArray())

    override fun decrypt(blob: String) = if (blob.startsWith("plain:")) String(java.util.Base64.getDecoder().decode(blob.removePrefix("plain:"))) else null
}

fun cat(name: String): JsonObject = buildJsonObject { put("name", name) }

fun rule(category: String): JsonObject = buildJsonObject { put("category", category) }

/** An id shaped like the ones devices make (43 characters of base64url), for records a test server makes up. */
private fun recordId(text: String): String =
    java.util.Base64.getUrlEncoder().withoutPadding().encodeToString(java.security.MessageDigest.getInstance("SHA-256").digest(text.toByteArray()))

/**
 * The account service against a server in memory: the cases of desktop's test/account.test.js, so both apps keep
 * the same sync, conflict and session rules.
 */
class AccountServiceTest {
    @get:Rule
    val tmp = TemporaryFolder()

    private val scopes = mutableListOf<CoroutineScope>()
    private val servers = mutableListOf<FakeAccountServer>()
    private val devices = mutableListOf<AccountService>()
    private val okhttp = OkHttpClient()

    @After
    fun tearDown() {
        devices.forEach { it.close() }
        scopes.forEach { it.cancel() }
        servers.forEach { it.close() }
    }

    private fun server(
        signups: String = "open",
        invites: List<String> = emptyList(),
        limit: Int = 1000,
        kdf: JsonObject = AccountKeys.DEFAULT_KDF.toJson(),
    ) = FakeAccountServer(signups, invites, limit, kdf = kdf).also { servers += it }

    private fun device(
        srv: FakeAccountServer?,
        name: String = "desk",
        dir: File = tmp.newFolder(),
        clock: Clock = Clock.System,
        http: AccountHttp = OkHttpAccountHttp(okhttp),
        pushAfterMs: Long = 60_000,
    ): AccountService {
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default).also { scopes += it }
        return AccountService(dir, PlainSecrets, srv?.url.orEmpty(), name, http, scope, devHosts = true, platform = "linux", clock = clock, pushAfterMs = pushAfterMs)
            .also { devices += it }
    }

    private suspend fun rejects(code: String, block: suspend () -> Unit) {
        try {
            block()
            fail("expected $code")
        } catch (e: AccountError) {
            assertEquals(code, e.code)
        }
    }

    /**
     * A device whose session ended on the server. A sync of its own may notice before the test asks (signing in starts
     * one), so this checks where it ends up rather than which sync noticed, as desktop's test does.
     */
    private suspend fun signedOut(device: AccountService) {
        // The sync joined may be the one signing in started, whose calls were made before the session ended.
        repeat(2) {
            try {
                device.sync()
            } catch (e: AccountError) {
                assertEquals("signed_out", e.code)
            }
        }
        assertFalse(device.status().signedIn)
        assertEquals("signed_out", device.status().error)
    }

    private fun AccountService.value(c: String, k: String): JsonElement? = get(c, k)

    @Test
    fun `signing up sends no password and nothing readable, and gives a recovery key`() = runBlocking {
        val srv = server()
        val desk = device(srv)
        desk.put("category", "needs-you", cat("Needs you"))
        val res = desk.signUp(" Bart@Example.com ", PASSWORD)
        assertNotNull(AccountKeys.parseRecoveryKey(res.recoveryKey))
        assertEquals("bart@example.com", desk.status().email)
        desk.sync()
        val sent = srv.calls.joinToString("\n") { it.body }
        assertFalse("the password never leaves the device", sent.contains(PASSWORD))
        assertFalse("records go up sealed", sent.contains("Needs you") || sent.contains("needs-you"))
        assertEquals(1, srv.accounts.getValue("bart@example.com").records.size)
        assertEquals(0, desk.status().pending)
        // The phone says what it is.
        assertTrue(srv.calls.first { it.route == "POST /v1/signup" }.body.contains("\"platform\":\"linux\""))
    }

    @Test
    fun `a sign-in proof made for one server address is no good at another`() = runBlocking {
        val srv = server()
        val desk = device(srv, "desk")
        val recoveryKey = desk.signUp("bound@example.com", PASSWORD).recoveryKey
        // The same server under another address, as a server that passes requests on would be.
        val relayed = device(null, "relayed").also { it.setServer(srv.url.replace("127.0.0.1", "localhost")) }
        rejects("unauthorized") { relayed.signIn("bound@example.com", PASSWORD) }
        rejects("unauthorized") { relayed.recover("bound@example.com", recoveryKey, "some new password") }
    }

    @Test
    fun `a second device signs in, gets the records and sends its own`() = runBlocking {
        val srv = server()
        val desk = device(srv, "desk")
        desk.signUp("b@example.com", PASSWORD)
        desk.put("category", "money", cat("Money"))
        desk.put("sender", "bank@example.com", rule("money"))
        desk.sync()

        val laptop = device(srv, "laptop")
        laptop.put("sender", "shop@example.com", rule("promotions"))
        laptop.signIn("B@example.com", PASSWORD)
        laptop.sync()
        assertEquals(cat("Money"), laptop.value("category", "money"))
        assertEquals(rule("money"), laptop.value("sender", "bank@example.com"))

        desk.sync()
        assertEquals("the laptop's own record went up", rule("promotions"), desk.value("sender", "shop@example.com"))
        assertEquals(listOf("desk", "laptop"), desk.details().devices.map { it.name }.sorted())
    }

    @Test
    fun `the newest change wins, a deletion travels, and the first classification stays`() = runBlocking {
        val srv = server()
        val desk = device(srv, "desk")
        desk.signUp("c@example.com", PASSWORD)
        val laptop = device(srv, "laptop")
        laptop.signIn("c@example.com", PASSWORD)

        desk.put("sender", "x@example.com", rule("updates"))
        desk.sync()
        laptop.sync()
        laptop.put("sender", "x@example.com", rule("needs-you"))
        laptop.sync()
        desk.sync()
        assertEquals(rule("needs-you"), desk.value("sender", "x@example.com"))

        desk.remove("sender", "x@example.com")
        desk.sync()
        laptop.sync()
        assertNull(laptop.value("sender", "x@example.com"))

        val key = "c@example.com\n<m1@x>"
        desk.put("classification", key, rule("money"), ifAbsent = true)
        laptop.put("classification", key, rule("updates"), ifAbsent = true)
        desk.sync()
        laptop.sync()
        assertEquals("the server kept the first one", rule("money"), laptop.value("classification", key))
        assertEquals(0, laptop.status().pending)
    }

    @Test
    fun `a first write that loses a race still brings the winner down`() = runBlocking {
        val srv = server()
        val desk = device(srv, "desk")
        desk.signUp("race@example.com", PASSWORD)
        val laptop = device(srv, "laptop")
        laptop.signIn("race@example.com", PASSWORD)
        desk.sync()
        laptop.sync()

        // Both have pulled; the desk's classification reaches the server first.
        val key = "race@example.com\n<m@x>"
        desk.put("classification", key, rule("money"), ifAbsent = true)
        laptop.put("classification", key, rule("updates"), ifAbsent = true)
        laptop.pull()
        desk.sync()
        laptop.push()
        laptop.sync()
        assertEquals(rule("money"), laptop.value("classification", key))
        desk.sync()
        assertEquals(rule("money"), desk.value("classification", key))
    }

    private class TestClock(var t: Long) : Clock {
        override fun now() = t
    }

    @Test
    fun `the later change wins whichever reaches the server last`() = runBlocking {
        val srv = server()
        val clock = TestClock(1000)
        val desk = device(srv, "desk", clock = clock)
        desk.signUp("lww@example.com", PASSWORD)
        val laptop = device(srv, "laptop", clock = clock)
        laptop.signIn("lww@example.com", PASSWORD)
        desk.sync()
        laptop.sync()

        // Both pull, the laptop changes the rule at 2000 and the desk at 3000, and the older change arrives last.
        desk.pull()
        laptop.pull()
        clock.t = 2000
        laptop.put("sender", "x@example.com", rule("updates"))
        clock.t = 3000
        desk.put("sender", "x@example.com", rule("needs-you"))
        desk.push()
        laptop.push()
        desk.sync()
        laptop.sync()
        assertEquals(rule("needs-you"), desk.value("sender", "x@example.com"))
        assertEquals(rule("needs-you"), laptop.value("sender", "x@example.com"))

        // A deletion at 4000 against a change at 5000 that arrives first: the change wins on both.
        desk.pull()
        laptop.pull()
        clock.t = 4000
        desk.remove("sender", "x@example.com")
        clock.t = 5000
        laptop.put("sender", "x@example.com", rule("money"))
        laptop.push()
        desk.push()
        laptop.sync()
        desk.sync()
        assertEquals(rule("money"), laptop.value("sender", "x@example.com"))
        assertEquals(rule("money"), desk.value("sender", "x@example.com"))

        // And a later deletion wins over an earlier change.
        clock.t = 6000
        laptop.put("sender", "x@example.com", rule("updates"))
        clock.t = 7000
        desk.remove("sender", "x@example.com")
        desk.push()
        laptop.push()
        desk.sync()
        laptop.sync()
        assertNull(desk.value("sender", "x@example.com"))
        assertNull(laptop.value("sender", "x@example.com"))
    }

    @Test
    fun `on the same time both devices keep the same version`() = runBlocking {
        val srv = server()
        val clock = TestClock(1000)
        val desk = device(srv, "desk", clock = clock)
        desk.signUp("tie@example.com", PASSWORD)
        val laptop = device(srv, "laptop", clock = clock)
        laptop.signIn("tie@example.com", PASSWORD)
        desk.sync()
        laptop.sync()
        clock.t = 9000
        desk.put("sender", "x@example.com", rule("money"))
        laptop.put("sender", "x@example.com", rule("updates"))
        laptop.push()
        desk.push()
        desk.sync()
        laptop.sync()
        desk.sync()
        // The larger JSON of [0, v]: "updates" sorts after "money".
        assertEquals(rule("updates"), desk.value("sender", "x@example.com"))
        assertEquals(rule("updates"), laptop.value("sender", "x@example.com"))
    }

    @Test
    fun `a deletion made while signed out goes up at the next sign-in`() = runBlocking {
        val srv = server()
        val dir = tmp.newFolder()
        val desk = device(srv, "desk", dir)
        desk.signUp("gone@example.com", PASSWORD)
        desk.put("category", "promotions", cat("Promotions"))
        desk.sync()
        desk.signOut()
        desk.remove("category", "promotions")
        desk.signIn("gone@example.com", PASSWORD)
        desk.sync()
        assertNull(desk.value("category", "promotions"))
        val laptop = device(srv, "laptop")
        laptop.signIn("gone@example.com", PASSWORD)
        laptop.sync()
        assertNull(laptop.value("category", "promotions"))
    }

    @Test
    fun `a sync still running for an account this device left changes nothing in the next one`() = runBlocking {
        val srv = server()
        val a = device(srv, "a-helper")
        a.signUp("first@example.com", PASSWORD)
        for (i in 0 until 5) a.put("sender", "a$i@example.com", rule("updates"))
        a.sync()
        val b = device(srv, "b-helper")
        b.signUp("second@example.com", PASSWORD)
        b.put("sender", "b0@example.com", rule("money"))
        b.sync()

        // The desk's first pull for the first account waits until the desk is in the second account.
        val held = CompletableDeferred<Unit>()
        var holding = true
        val real = OkHttpAccountHttp(okhttp)
        val desk = device(srv, "desk", http = AccountHttp { server, method, path, body, token, meter ->
            if (holding && path.startsWith("/v1/records?")) {
                holding = false
                held.await()
            }
            real.request(server, method, path, body, token, meter)
        })
        desk.signIn("first@example.com", PASSWORD)
        val old = desk.running()!!
        desk.signOut()
        desk.signIn("second@example.com", PASSWORD)
        desk.sync()
        held.complete(Unit)
        runCatching { old.await() }
        assertEquals(rule("money"), desk.value("sender", "b0@example.com"))

        b.put("sender", "b1@example.com", rule("to-read"))
        b.sync()
        desk.sync()
        assertEquals("the old answer moved no cursor", rule("to-read"), desk.value("sender", "b1@example.com"))
        assertNull(desk.value("sender", "a0@example.com"))
        assertTrue(desk.status().signedIn)
    }

    @Test
    fun `a slow sign-out leaves alone the session made while it waited`() = runBlocking {
        val srv = server()
        val helper = device(srv, "helper")
        helper.signUp("slow@example.com", PASSWORD)
        val held = CompletableDeferred<Unit>()
        val hold = java.util.concurrent.atomic.AtomicBoolean(false)
        val waiting = CompletableDeferred<Unit>()
        val real = OkHttpAccountHttp(okhttp)
        val desk = device(srv, "desk", http = AccountHttp { server, method, path, body, token, meter ->
            if (path == "/v1/logout" && hold.getAndSet(false)) {
                waiting.complete(Unit)
                held.await()
            }
            real.request(server, method, path, body, token, meter)
        })
        desk.signIn("slow@example.com", PASSWORD)
        desk.sync()
        hold.set(true)
        val leaving = async { desk.signOut() }
        waiting.await()
        // Meanwhile the session ends another way (another device signs this one out), and the person signs in again.
        helper.removeDevice(helper.details().devices.first { it.name == "desk" }.id)
        signedOut(desk)
        desk.signIn("slow@example.com", PASSWORD)
        held.complete(Unit)
        leaving.await()
        assertTrue(desk.status().signedIn)
    }

    @Test
    fun `a classification deleted while signed out stays deleted when signing in`() = runBlocking {
        val srv = server()
        val clock = TestClock(100)
        val helper = device(srv, "helper", clock = clock)
        helper.signUp("tomb@example.com", PASSWORD)
        val key = "tomb@example.com\n<old@x>"
        helper.put("classification", key, rule("updates"), ifAbsent = true)
        helper.sync()
        val desk = device(srv, "desk", clock = clock)
        desk.signIn("tomb@example.com", PASSWORD)
        desk.sync()
        desk.signOut()
        clock.t = 201
        desk.remove("classification", key)
        desk.signIn("tomb@example.com", PASSWORD)
        desk.sync()
        assertNull(desk.value("classification", key))
        helper.sync()
        assertNull("the deletion reached the other device", helper.value("classification", key))
    }

    @Test
    fun `a classification made again after its deletion went round stays`() = runBlocking {
        val srv = server()
        val desk = device(srv, "desk")
        desk.signUp("again@example.com", PASSWORD)
        val laptop = device(srv, "laptop")
        laptop.signIn("again@example.com", PASSWORD)
        val key = "again@example.com\n<m@x>"
        desk.put("classification", key, rule("money"), ifAbsent = true)
        desk.sync()
        desk.remove("classification", key)
        desk.sync()
        assertTrue(desk.put("classification", key, rule("updates"), ifAbsent = true))
        desk.sync()
        assertEquals(rule("updates"), desk.value("classification", key))
        assertEquals(0, desk.status().pending)
        laptop.sync()
        assertEquals(rule("updates"), laptop.value("classification", key))
    }

    @Test
    fun `a tombstone from the server erases nothing, since it proves nothing`() = runBlocking {
        val srv = server()
        val desk = device(srv, "desk")
        desk.signUp("tomb2@example.com", PASSWORD)
        desk.put("sender", "keep@example.com", rule("money"))
        desk.sync()
        val laptop = device(srv, "laptop")
        laptop.signIn("tomb2@example.com", PASSWORD)
        laptop.sync()
        // The server swaps the record for a tombstone of its own.
        val stored = srv.accounts.getValue("tomb2@example.com")
        synchronized(stored) {
            val id = stored.records.keys.first()
            stored.seq += 1
            stored.records[id] = FakeAccountServer.Stored(stored.seq, null)
        }
        laptop.sync()
        assertEquals(rule("money"), laptop.value("sender", "keep@example.com"))
    }

    @Test
    fun `a false sequence number from the server makes the device start over, not skip records`() = runBlocking {
        val srv = server()
        val desk = device(srv, "desk")
        desk.signUp("seq@example.com", PASSWORD)
        desk.put("sender", "a@example.com", rule("money"))
        desk.sync()
        // One answer claims a change far ahead of the server's.
        val lie = java.util.concurrent.atomic.AtomicBoolean(true)
        val real = OkHttpAccountHttp(okhttp)
        val laptop = device(srv, "laptop", http = AccountHttp { server, method, path, body, token, meter ->
            val res = real.request(server, method, path, body, token, meter)
            if (path.startsWith("/v1/records?") && lie.getAndSet(false)) {
                val o = res as JsonObject
                buildJsonObject {
                    put("records", kotlinx.serialization.json.JsonArray((o["records"] as kotlinx.serialization.json.JsonArray) + buildJsonObject {
                        put("id", "x".repeat(43))
                        put("seq", 999999)
                        put("data", kotlinx.serialization.json.JsonNull)
                    }))
                    put("seq", 999999)
                    put("more", false)
                }
            } else {
                res
            }
        })
        laptop.signIn("seq@example.com", PASSWORD)
        laptop.sync()
        desk.put("sender", "b@example.com", rule("updates"))
        desk.sync()
        laptop.sync()
        assertEquals("nothing was skipped", rule("updates"), laptop.value("sender", "b@example.com"))
        assertTrue(laptop.cursor() < 999999)
    }

    @Test
    fun `a page out of order is refused whole and moves nothing`() = runBlocking {
        val srv = server()
        val real = OkHttpAccountHttp(okhttp)
        val desk = device(srv, "desk", http = AccountHttp { server, method, path, body, token, meter ->
            val res = real.request(server, method, path, body, token, meter)
            val records = (res as? JsonObject)?.get("records") as? kotlinx.serialization.json.JsonArray
            if (path.startsWith("/v1/records?") && !records.isNullOrEmpty()) {
                JsonObject(res + mapOf("records" to kotlinx.serialization.json.JsonArray(records.reversed()), "seq" to kotlinx.serialization.json.JsonPrimitive(1)))
            } else {
                res
            }
        })
        val helper = device(srv, "helper")
        helper.signUp("order@example.com", PASSWORD)
        helper.put("sender", "a@example.com", rule("money"))
        helper.put("sender", "b@example.com", rule("money"))
        helper.sync()
        desk.signIn("order@example.com", PASSWORD)
        rejects("server") { desk.sync() }
        assertEquals(0L, desk.cursor())
        assertNull(desk.value("sender", "a@example.com"))
    }

    @Test
    fun `a record too large for the server stays here and doesn't hold up the rest`() = runBlocking {
        val srv = server()
        val desk = device(srv, "desk")
        desk.signUp("big@example.com", PASSWORD)
        val huge = buildJsonObject { put("note", "x".repeat(20000)) }
        desk.put("sender", "huge@example.com", huge)
        desk.put("sender", "fine@example.com", rule("money"))
        desk.sync()
        assertEquals(0, desk.status().pending)
        assertEquals("kept on this phone", huge, desk.value("sender", "huge@example.com"))
        val laptop = device(srv, "laptop")
        laptop.signIn("big@example.com", PASSWORD)
        laptop.sync()
        assertEquals(rule("money"), laptop.value("sender", "fine@example.com"))
        assertNull(laptop.value("sender", "huge@example.com"))
    }

    @Test
    fun `signing in with a wrong password fails without signing in`() = runBlocking {
        val srv = server()
        val desk = device(srv)
        desk.signUp("d@example.com", PASSWORD)
        val other = device(srv, "other")
        rejects("unauthorized") { other.signIn("d@example.com", "not the password!") }
        rejects("unauthorized") { other.signIn("nobody@example.com", PASSWORD) }
        assertFalse(other.status().signedIn)
    }

    @Test
    fun `sign-up checks the address and the password length first`() = runBlocking {
        val srv = server()
        val desk = device(srv)
        rejects("invalid_email") { desk.signUp("not an address", PASSWORD) }
        rejects("weak_password") { desk.signUp("e@example.com", "short") }
        // Twelve characters are counted as characters, not as UTF-16 units: eleven emoji are too short.
        rejects("weak_password") { desk.signUp("e@example.com", "😀".repeat(11)) }
        assertEquals(0, srv.calls.size)
    }

    @Test
    fun `a server that hands out a weak password hash is refused`() = runBlocking {
        val srv = server(kdf = buildJsonObject {
            put("alg", "argon2id")
            put("m", 1024)
            put("t", 1)
            put("p", 1)
        })
        val desk = device(srv)
        rejects("weak_kdf") { desk.signIn("f@example.com", PASSWORD) }
        assertFalse("no key for the password went out", srv.calls.any { it.route == "POST /v1/login" })
    }

    @Test
    fun `a server that asks for more work than a phone can spare is refused before hashing`() = runBlocking {
        // Both within the protocol's limits, which a desktop can afford: 1 GiB and ten passes, and 256 MiB (which the
        // test's heap holds) with five passes, which would take a phone a long while.
        for ((m, t) in listOf(1048576 to 10, 262144 to 5)) {
            val srv = server(kdf = buildJsonObject {
                put("alg", "argon2id")
                put("m", m)
                put("t", t)
                put("p", 4)
            })
            val desk = device(srv)
            val started = System.nanoTime()
            rejects("heavy_kdf") { desk.signIn("f@example.com", PASSWORD) }
            assertTrue("refused without running the hash", System.nanoTime() - started < 2_000_000_000L)
            assertFalse("no key for the password went out", srv.calls.any { it.route == "POST /v1/login" })
        }
    }

    @Test
    fun `invite-only and closed servers say so`() = runBlocking {
        val invite = server(signups = "invite", invites = listOf("WELCOME"))
        val a = device(invite)
        rejects("invite_required") { a.signUp("g@example.com", PASSWORD) }
        a.signUp("g@example.com", PASSWORD, invite = " WELCOME ")
        assertTrue(a.status().signedIn)
        val closed = server(signups = "closed")
        rejects("signups_closed") { device(closed).signUp("h@example.com", PASSWORD) }
    }

    @Test
    fun `a changed password signs other devices out, and a typo does not sign this one out`() = runBlocking {
        val srv = server()
        val desk = device(srv, "desk")
        desk.signUp("i@example.com", PASSWORD)
        desk.put("category", "to-read", cat("To read"))
        desk.sync()
        val laptop = device(srv, "laptop")
        laptop.signIn("i@example.com", PASSWORD)

        rejects("wrong_password") { desk.changePassword("wrong password!!", "another long password") }
        assertTrue(desk.status().signedIn)
        desk.changePassword(PASSWORD, "another long password")
        signedOut(laptop)

        val phone = device(srv, "phone")
        phone.signIn("i@example.com", "another long password")
        phone.sync()
        assertEquals("the same data key under the new password", cat("To read"), phone.value("category", "to-read"))
    }

    @Test
    fun `the recovery key sets a new password and keeps the data`() = runBlocking {
        val srv = server()
        val desk = device(srv, "desk")
        val recoveryKey = desk.signUp("j@example.com", PASSWORD).recoveryKey
        desk.put("sender", "boss@example.com", rule("needs-you"))
        desk.sync()

        val phone = device(srv, "phone")
        rejects("recovery_key") { phone.recover("j@example.com", "nope", "brand new password") }
        rejects("unauthorized") { phone.recover("j@example.com", AccountKeys.formatRecoveryKey(AccountKeys.randomBytes(20)), "brand new password") }
        phone.recover("j@example.com", recoveryKey.lowercase(), "brand new password")
        phone.sync()
        assertEquals(rule("needs-you"), phone.value("sender", "boss@example.com"))
        signedOut(desk)

        val next = phone.newRecoveryKey("brand new password")
        assertNotEquals(recoveryKey, next)
        phone.signOut()
        rejects("unauthorized") { phone.recover("j@example.com", recoveryKey, "yet another password") }
    }

    @Test
    fun `signing in again on this phone survives a restart, and signing out keeps local data`() = runBlocking {
        val srv = server()
        val dir = tmp.newFolder()
        val first = device(srv, "desk", dir)
        first.signUp("k@example.com", PASSWORD)
        first.put("category", "updates", cat("Updates"))
        first.sync()
        first.close()

        val again = device(srv, "desk", dir)
        assertTrue(again.status().signedIn)
        assertEquals(cat("Updates"), again.value("category", "updates"))
        again.put("category", "promotions", cat("Promotions"))
        again.sync()
        assertEquals(2, srv.accounts.getValue("k@example.com").records.size)

        again.signOut()
        assertFalse(again.status().signedIn)
        assertEquals(cat("Promotions"), again.value("category", "promotions"))
        val saved = File(dir, "account.json").readText()
        assertFalse("no session left on disk", saved.contains("token") || saved.contains("dataKey"))
    }

    @Test
    fun `a session stored under a Keystore key this phone lost signs in again`() = runBlocking {
        val srv = server()
        val dir = tmp.newFolder()
        val first = device(srv, "desk", dir)
        first.signUp("lost@example.com", PASSWORD)
        first.put("category", "updates", cat("Updates"))
        first.sync()
        first.close()
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default).also { scopes += it }
        val forgetful = object : SecretCipher {
            override fun encrypt(plain: String) = PlainSecrets.encrypt(plain)

            override fun decrypt(blob: String): String? = null
        }
        val again = AccountService(dir, forgetful, srv.url, "desk", OkHttpAccountHttp(okhttp), scope, devHosts = true).also { devices += it }
        assertFalse(again.status().signedIn)
        assertEquals("signed_out", again.status().error)
        assertEquals("its records stay", cat("Updates"), again.value("category", "updates"))
    }

    /**
     * A Rukoo whose default server is [defaultServer], as one release or the next would have it. Rukoo's former and
     * current default both reach [srv], as the real server answers on both.
     */
    private fun release(srv: FakeAccountServer, defaultServer: String, dir: File, name: String = "desk"): AccountService {
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default).also { scopes += it }
        val real = OkHttpAccountHttp(okhttp)
        val http = AccountHttp { server, method, path, body, token, meter ->
            real.request(if (server == OLD_DEFAULT || server == NEW_DEFAULT) srv.url else server, method, path, body, token, meter)
        }
        return AccountService(dir, PlainSecrets, defaultServer, name, http, scope, devHosts = true, platform = "linux").also { devices += it }
    }

    private fun stateOnDisk(dir: File) = Json.parseToJsonElement(File(dir, "account.json").readText()) as JsonObject

    @Test
    fun `a phone signed in at the old default keeps that address when the default moves, until it signs out`() = runBlocking {
        val srv = server()
        val dir = tmp.newFolder()
        val before = release(srv, OLD_DEFAULT, dir)
        before.signUp("pin@example.com", PASSWORD)
        before.put("category", "updates", cat("Updates"))
        before.sync()
        before.close()
        assertNull("the default is not stored", stateOnDisk(dir)["server"]?.takeIf { it != JsonNull })
        // As an older Rukoo wrote it, without the session's address: the server info fetched at sign-in names it.
        val older = stateOnDisk(dir).toMutableMap()
        older.remove("sessionServer")
        assertEquals(OLD_DEFAULT, ((older["info"] as JsonObject)["server"] as JsonPrimitive).content)
        File(dir, "account.json").writeText(JsonObject(older).toString())

        // The update: the same session, still at the address its password proofs were made for.
        val after = release(srv, NEW_DEFAULT, dir)
        assertTrue(after.status().signedIn)
        assertEquals(OLD_DEFAULT, after.status().server)
        assertEquals(NEW_DEFAULT, after.status().defaultServer)
        assertEquals(JsonPrimitive(OLD_DEFAULT), stateOnDisk(dir)["server"])
        assertEquals("Test operator", after.status().info?.operator?.name)
        after.put("category", "promotions", cat("Promotions"))
        after.sync()
        assertEquals(2, srv.accounts.getValue("pin@example.com").records.size)
        after.changePassword(PASSWORD, "another long password")
        assertTrue(after.newRecoveryKey("another long password").isNotEmpty())
        // A phone at the new default can't sign in to it: the proof is made for the other address.
        val fresh = release(srv, NEW_DEFAULT, tmp.newFolder(), "laptop")
        rejects("unauthorized") { fresh.signIn("pin@example.com", "another long password") }
        after.close()

        // Restarts keep it, now from the stored address.
        val again = release(srv, NEW_DEFAULT, dir)
        assertEquals(OLD_DEFAULT, again.status().server)
        again.signOut()
        assertFalse(again.status().signedIn)
        assertEquals("signed out, it uses the default", NEW_DEFAULT, again.status().server)
        assertNull(stateOnDisk(dir)["server"]?.takeIf { it != JsonNull })
        assertNull(again.status().info)
        assertEquals(cat("Promotions"), again.value("category", "promotions"))
        // The account itself is still at the old address, where it can be signed in to by hand.
        rejects("unauthorized") { again.signIn("pin@example.com", "another long password") }
        again.setServer(OLD_DEFAULT)
        again.signIn("pin@example.com", "another long password")
        assertTrue(again.status().signedIn)
    }

    @Test
    fun `deleting the account on a pinned phone moves it to the default, where the account can be made again`() = runBlocking {
        val srv = server()
        val dir = tmp.newFolder()
        val before = release(srv, OLD_DEFAULT, dir)
        before.signUp("move@example.com", PASSWORD)
        before.close()

        val after = release(srv, NEW_DEFAULT, dir)
        assertEquals(OLD_DEFAULT, after.status().server)
        after.deleteAccount(PASSWORD)
        assertEquals(NEW_DEFAULT, after.status().server)
        assertNull(stateOnDisk(dir)["server"]?.takeIf { it != JsonNull })
        after.signUp("move@example.com", PASSWORD)
        assertEquals(NEW_DEFAULT, after.status().server)
        after.close()

        // Signed in at the default of the day: nothing to pin.
        val again = release(srv, NEW_DEFAULT, dir)
        assertTrue(again.status().signedIn)
        assertNull(stateOnDisk(dir)["server"]?.takeIf { it != JsonNull })
        assertTrue(again.changePassword(PASSWORD, "another long password"))
    }

    @Test
    fun `a pinned address that was never a default stays, and a former default ends with the session, also one ended elsewhere`() = runBlocking {
        val srv = server()
        // A fork whose default moved: its old address isn't one of Rukoo's, so signing out keeps it.
        val dir = tmp.newFolder()
        val first = device(srv, "desk", dir)
        first.signUp("fork@example.com", PASSWORD)
        first.close()
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default).also { scopes += it }
        val other = srv.url.replace("127.0.0.1", "localhost")
        val moved = AccountService(dir, PlainSecrets, other, "desk", OkHttpAccountHttp(okhttp), scope, devHosts = true, platform = "linux").also { devices += it }
        assertEquals(srv.url, moved.status().server)
        moved.signOut()
        assertEquals(srv.url, moved.status().server)
        moved.signIn("fork@example.com", PASSWORD)

        // The way to move: one device deletes the account and makes it again at the default. The phone, pinned as
        // well, finds its session ended and goes to the default too, where the account is now.
        val deskDir = tmp.newFolder()
        val phoneDir = tmp.newFolder()
        val desk = release(srv, OLD_DEFAULT, deskDir, "desk")
        desk.signUp("moved@example.com", PASSWORD)
        desk.close()
        val phone = release(srv, OLD_DEFAULT, phoneDir, "phone")
        phone.signIn("moved@example.com", PASSWORD)
        phone.close()
        val newDesk = release(srv, NEW_DEFAULT, deskDir, "desk")
        val newPhone = release(srv, NEW_DEFAULT, phoneDir, "phone")
        assertEquals(OLD_DEFAULT, newDesk.status().server)
        assertEquals(OLD_DEFAULT, newPhone.status().server)
        newDesk.deleteAccount(PASSWORD)
        newDesk.signUp("moved@example.com", PASSWORD)
        signedOut(newPhone)
        assertEquals(NEW_DEFAULT, newPhone.status().server)
        assertNull(stateOnDisk(phoneDir)["server"]?.takeIf { it != JsonNull })
        newPhone.signIn("moved@example.com", PASSWORD)
        assertTrue(newPhone.status().signedIn)
    }

    @Test
    fun `a phone signed out at the old default follows the new one`() = runBlocking {
        val srv = server()
        val dir = tmp.newFolder()
        val before = release(srv, OLD_DEFAULT, dir)
        before.serverInfo()
        before.close()
        val after = release(srv, NEW_DEFAULT, dir)
        assertEquals(NEW_DEFAULT, after.status().server)
        assertNull("the old address's info isn't shown for the new one", after.status().info)
        after.signUp("new@example.com", PASSWORD)
        assertEquals(NEW_DEFAULT, after.status().server)
    }

    @Test
    fun `a device that was removed elsewhere finds itself signed out`() = runBlocking {
        val srv = server()
        val desk = device(srv, "desk")
        desk.signUp("l@example.com", PASSWORD)
        val laptop = device(srv, "laptop")
        laptop.signIn("l@example.com", PASSWORD)
        val devices = desk.details().devices
        desk.removeDevice(devices.first { it.name == "laptop" }.id)
        signedOut(laptop)
    }

    @Test
    fun `deleting the account needs the password and clears the server`() = runBlocking {
        val srv = server()
        val desk = device(srv)
        desk.signUp("m@example.com", PASSWORD)
        rejects("wrong_password") { desk.deleteAccount("wrong password!!") }
        assertTrue("a wrong password is no lost session", desk.status().signedIn)
        desk.deleteAccount(PASSWORD)
        assertFalse(srv.accounts.containsKey("m@example.com"))
        assertFalse(desk.status().signedIn)
    }

    @Test
    fun `many records go up and come down in pages`() = runBlocking {
        val srv = server(limit = 50)
        val desk = device(srv, "desk")
        desk.signUp("n@example.com", PASSWORD)
        for (i in 0 until 620) desk.put("classification", "n@example.com\n<$i@x>", rule("updates"), ifAbsent = true)
        desk.sync()
        val laptop = device(srv, "laptop")
        laptop.signIn("n@example.com", PASSWORD)
        laptop.sync()
        assertEquals(620, laptop.list("classification").size)
        assertEquals(620, laptop.status().counts["classification"])
    }

    @Test
    fun `records taken before a later page failed are announced`() = runBlocking {
        val srv = server(limit = 2)
        val desk = device(srv, "desk")
        desk.signUp("pages@example.com", PASSWORD)
        val failPage = AtomicInteger()
        val pages = AtomicInteger()
        val base = OkHttpAccountHttp(okhttp)
        val flaky = AccountHttp { server, method, path, body, token, meter ->
            if (method == "GET" && path.startsWith("/v1/records") && pages.incrementAndGet() == failPage.get()) throw AccountError("network")
            base.request(server, method, path, body, token, meter)
        }
        val laptop = device(srv, "laptop", http = flaky)
        laptop.signIn("pages@example.com", PASSWORD)
        laptop.sync()
        for (i in 1..5) desk.put("category", "c$i", cat("C$i"))
        desk.sync()

        // The first page (two records) comes down, the second fails.
        pages.set(0)
        failPage.set(2)
        val before = laptop.version.value
        rejects("network") { laptop.sync() }
        assertEquals(2, laptop.list("category").size)
        assertTrue("the records taken are announced", laptop.version.value > before)
        failPage.set(0)
        laptop.sync()
        assertEquals(5, laptop.list("category").size)
    }

    @Test
    fun `a server that always has more can't keep a pull going`() = runBlocking {
        val srv = server()
        val pages = AtomicInteger()
        // 0: the real server; 1: a page of one more tombstone each time, forever; 2: more records than asked for.
        val mode = AtomicInteger()
        val base = OkHttpAccountHttp(okhttp)
        val http = AccountHttp { server, method, path, body, token, meter ->
            if (mode.get() == 0 || method != "GET" || !path.startsWith("/v1/records")) return@AccountHttp base.request(server, method, path, body, token, meter)
            val since = path.substringAfter("since=").substringBefore('&').toLong()
            val n = pages.incrementAndGet()
            buildJsonObject {
                putJsonArray("records") {
                    repeat(if (mode.get() == 1) 1 else 1001) { i ->
                        addJsonObject {
                            put("id", recordId("$n-$i"))
                            put("seq", since + 1 + i)
                            put("data", JsonNull)
                        }
                    }
                }
                put("seq", 1_000_000_000L)
                // Not quite forever, so that a phone without a limit fails the test rather than hang it.
                put("more", n < 2 * AccountService.PULL_PAGES)
            }
        }
        val desk = device(srv, http = http)
        desk.signUp("endless@example.com", PASSWORD)
        desk.sync()
        mode.set(1)
        desk.pull()
        assertEquals(AccountService.PULL_PAGES, pages.get())
        mode.set(2)
        rejects("server") { desk.pull() }
    }

    @Test
    fun `a server that pads its answers can't get past the bytes one pull may take`() = runBlocking {
        val srv = server()
        // Each page is one tombstone and 4 MiB of a field Rukoo ignores, from a server of its own, so every byte goes
        // through the real client. The data the old limit counted stays empty.
        val pad = "x".repeat(4 * 1024 * 1024)
        val pages = AtomicInteger()
        val sent = AtomicLong()
        val last = AtomicLong()
        val padded = MockWebServer()
        padded.dispatcher = object : Dispatcher() {
            override fun dispatch(request: RecordedRequest): MockResponse {
                val since = request.requestUrl!!.queryParameter("since")!!.toLong()
                val n = pages.incrementAndGet()
                val body = buildJsonObject {
                    putJsonArray("records") {
                        addJsonObject {
                            put("id", recordId("$n"))
                            put("seq", since + 1)
                            put("data", JsonNull)
                        }
                    }
                    put("seq", 1_000_000_000L)
                    put("more", n < 2 * AccountService.PULL_PAGES)
                    put("pad", pad)
                }.toString()
                last.set(body.toByteArray().size.toLong())
                sent.addAndGet(last.get())
                return MockResponse().setHeader("Content-Type", "application/json").setBody(body)
            }
        }
        padded.start()
        try {
            val paddedUrl = "http://127.0.0.1:${padded.port}"
            val on = AtomicBoolean(false)
            val base = OkHttpAccountHttp(okhttp)
            val http = AccountHttp { server, method, path, body, token, meter ->
                val records = on.get() && method == "GET" && path.startsWith("/v1/records")
                base.request(if (records) paddedUrl else server, method, path, body, token, meter)
            }
            val desk = device(srv, http = http)
            desk.signUp("padded@example.com", PASSWORD)
            desk.sync()
            on.set(true)
            desk.pull()
            assertTrue("stopped by bytes, not pages (${pages.get()} pages)", pages.get() < AccountService.PULL_PAGES)
            assertTrue("the pull ends with the first page past the limit", sent.get() >= AccountService.PULL_DATA && sent.get() - last.get() < AccountService.PULL_DATA)
            assertEquals("the pages taken still count; the rest waits for the next sync", pages.get().toLong(), desk.cursor())
        } finally {
            padded.shutdown()
        }
    }

    @Test
    fun `a server that pads its answers with chunk extensions can't get past the bytes one pull may take`() = runBlocking {
        val srv = server()
        // Each page is one tombstone behind 4 MiB of chunk extension: HTTP framing, which OkHttp reads and drops before
        // the body. Counting bodies alone, the pull would run to 500 pages, 2 GiB.
        val pages = AtomicInteger()
        val last = AtomicLong()
        val padded = RawHttpServer { target, out ->
            val since = target.substringAfter("since=").substringBefore('&').toLong()
            val n = pages.incrementAndGet()
            val body = buildJsonObject {
                putJsonArray("records") {
                    addJsonObject {
                        put("id", recordId("$n"))
                        put("seq", since + 1)
                        put("data", JsonNull)
                    }
                }
                put("seq", 1_000_000_000L)
                put("more", n < 2 * AccountService.PULL_PAGES)
            }.toString().toByteArray()
            last.set(RawHttpServer.chunked(out, body, 4L * 1024 * 1024))
        }
        padded.use {
            val on = AtomicBoolean(false)
            val base = OkHttpAccountHttp(okhttp)
            val http = AccountHttp { server, method, path, body, token, meter ->
                val records = on.get() && method == "GET" && path.startsWith("/v1/records")
                base.request(if (records) padded.url else server, method, path, body, token, meter)
            }
            val desk = device(srv, http = http)
            desk.signUp("chunked@example.com", PASSWORD)
            desk.sync()
            on.set(true)
            desk.pull()
            val sent = padded.written.get()
            assertTrue("stopped by bytes, not pages (${pages.get()} pages)", pages.get() < AccountService.PULL_PAGES)
            assertTrue("the pull ends with the first page past the limit ($sent bytes)", sent >= AccountService.PULL_DATA && sent - last.get() < AccountService.PULL_DATA)
            assertEquals("one connection per page", pages.get(), padded.connections.get())
            assertEquals("the pages taken still count; the rest waits for the next sync", pages.get().toLong(), desk.cursor())
        }
    }

    @Test
    fun `a server that hides padding in compressed answers can't get past the bytes one pull may take`() = runBlocking {
        val srv = server()
        // Each page is a few bytes of JSON in gzip, behind 4 MiB of padding in the gzip header's file name: tiny once
        // unpacked, 4 MiB to download. The server sends it compressed whatever the phone asks for.
        val pages = AtomicInteger()
        val asked = java.util.concurrent.CopyOnWriteArrayList<String?>()
        val padded = MockWebServer()
        padded.dispatcher = object : Dispatcher() {
            override fun dispatch(request: RecordedRequest): MockResponse {
                asked += request.getHeader("Accept-Encoding")
                val since = request.requestUrl!!.queryParameter("since")!!.toLong()
                val n = pages.incrementAndGet()
                val json = buildJsonObject {
                    putJsonArray("records") {
                        addJsonObject {
                            put("id", recordId("$n"))
                            put("seq", since + 1)
                            put("data", JsonNull)
                        }
                    }
                    put("seq", 1_000_000_000L)
                    put("more", n < 2 * AccountService.PULL_PAGES)
                }.toString()
                val gzip = java.io.ByteArrayOutputStream().also { out -> java.util.zip.GZIPOutputStream(out).use { it.write(json.toByteArray()) } }.toByteArray()
                // FNAME: a file name follows the 10-byte header, up to a zero byte.
                val head = gzip.copyOfRange(0, 10).also { it[3] = (it[3].toInt() or 0x08).toByte() }
                val body = head + ByteArray(4 * 1024 * 1024) { 'x'.code.toByte() } + byteArrayOf(0) + gzip.copyOfRange(10, gzip.size)
                return MockResponse().setHeader("Content-Type", "application/json").setHeader("Content-Encoding", "gzip").setBody(okio.Buffer().write(body))
            }
        }
        padded.start()
        try {
            val paddedUrl = "http://127.0.0.1:${padded.port}"
            val on = AtomicBoolean(false)
            val base = OkHttpAccountHttp(okhttp)
            val http = AccountHttp { server, method, path, body, token, meter ->
                val records = on.get() && method == "GET" && path.startsWith("/v1/records")
                base.request(if (records) paddedUrl else server, method, path, body, token, meter)
            }
            val desk = device(srv, http = http)
            desk.signUp("gzip@example.com", PASSWORD)
            desk.sync()
            val before = desk.cursor()
            on.set(true)
            val error = runCatching { desk.pull() }.exceptionOrNull()
            assertEquals("the first compressed answer ends the pull", 1, pages.get())
            assertEquals("server", (error as? AccountError)?.code)
            assertEquals("the phone asks for answers as they are", listOf<String?>("identity"), asked.toList())
            assertEquals("and moves nothing", before, desk.cursor())
        } finally {
            padded.shutdown()
        }
    }

    @Test
    fun `a page with a record id no device could have made is refused whole and moves nothing`() = runBlocking {
        val srv = server()
        val helper = device(srv, "helper")
        helper.signUp("ids@example.com", PASSWORD)
        helper.put("sender", "a@example.com", rule("money"))
        helper.sync()
        // After the real record, a tombstone with an odd id, while the server lies.
        val lying = AtomicBoolean(true)
        val bad = AtomicReference<JsonElement>()
        val real = OkHttpAccountHttp(okhttp)
        val desk = device(srv, "desk", http = AccountHttp { server, method, path, body, token, meter ->
            val res = real.request(server, method, path, body, token, meter)
            if (!lying.get() || !path.startsWith("/v1/records?")) return@AccountHttp res
            val o = res as JsonObject
            val top = (o["seq"] as JsonPrimitive).content.toLong() + 1
            val odd = buildJsonObject {
                put("id", bad.get())
                put("seq", top)
                put("data", JsonNull)
            }
            JsonObject(o + mapOf("records" to JsonArray((o["records"] as JsonArray) + odd), "seq" to JsonPrimitive(top)))
        })
        // Too long (as padding would be), too short, a character outside base64url, padded base64, and not a string.
        val odd = listOf(
            JsonPrimitive("A".repeat(44)), JsonPrimitive("A".repeat(1_000_000)), JsonPrimitive("A".repeat(42)), JsonPrimitive("A".repeat(42) + "+"),
            JsonPrimitive("A".repeat(41) + "=="), JsonPrimitive(42), JsonNull, JsonArray(listOf(JsonPrimitive("A".repeat(43)))),
        )
        bad.set(odd.first())
        desk.signIn("ids@example.com", PASSWORD)
        for (id in odd) {
            bad.set(id)
            rejects("server") { desk.sync() }
            assertEquals(0L, desk.cursor())
            assertNull(desk.value("sender", "a@example.com"))
        }
        lying.set(false)
        desk.sync()
        assertEquals(rule("money"), desk.value("sender", "a@example.com"))
    }

    @Test
    fun `the cursor on disk never runs ahead of the records on disk`() = runBlocking {
        val srv = server(limit = 2)
        val desk = device(srv, "desk")
        desk.signUp("disk@example.com", PASSWORD)
        val pages = AtomicInteger()
        val trap = AtomicBoolean(false)
        val second = CompletableDeferred<Unit>()
        val base = OkHttpAccountHttp(okhttp)
        val http = AccountHttp { server, method, path, body, token, meter ->
            if (trap.get() && method == "GET" && path.startsWith("/v1/records") && pages.incrementAndGet() == 2) {
                // The second page never comes: Android stops the app while it waits.
                second.complete(Unit)
                CompletableDeferred<Unit>().await()
            }
            base.request(server, method, path, body, token, meter)
        }
        val dir = tmp.newFolder()
        val laptop = device(srv, "laptop", dir = dir, http = http)
        laptop.signIn("disk@example.com", PASSWORD)
        laptop.sync()
        for (i in 1..5) desk.put("category", "c$i", cat("C$i"))
        desk.sync()

        trap.set(true)
        val stuck = scopes.first().async { laptop.sync() }
        withTimeout(10_000) { second.await() }
        // The first page is taken; something saves the account meanwhile (the account page asks who runs the server).
        laptop.serverInfo()
        stuck.cancel()
        // The app starts again, offline, so what it shows is what was on disk.
        val fresh = device(srv, "fresh", dir = dir, http = AccountHttp { _, _, _, _, _, _ -> throw AccountError("network") })
        assertTrue("the cursor moved", fresh.cursor() > 0)
        assertEquals("the records it moved past are on disk too", 2, fresh.list("category").size)
    }

    @Test
    fun `a server that leaves records out of its answer doesn't get them again forever`() = runBlocking {
        val srv = server()
        val posts = AtomicInteger()
        val empty = AtomicBoolean(false)
        val base = OkHttpAccountHttp(okhttp)
        val http = AccountHttp { server, method, path, body, token, meter ->
            // Not quite forever, so that a phone without the check fails the test rather than hang it.
            if (empty.get() && method == "POST" && path == "/v1/records" && posts.incrementAndGet() <= 20) {
                return@AccountHttp buildJsonObject {
                    putJsonArray("results") {}
                    put("seq", 0)
                }
            }
            base.request(server, method, path, body, token, meter)
        }
        val desk = device(srv, http = http)
        desk.signUp("acks@example.com", PASSWORD)
        desk.sync()
        desk.put("category", "money", cat("Money"))
        empty.set(true)
        rejects("server") { desk.sync() }
        assertEquals(1, posts.get())
    }

    @Test
    fun `records the disk refuses keep the cursor on disk behind them`() = runBlocking {
        val srv = server()
        val desk = device(srv, "desk")
        desk.signUp("full@example.com", PASSWORD)
        val dir = tmp.newFolder()
        val laptop = device(srv, "laptop", dir = dir)
        laptop.signIn("full@example.com", PASSWORD)
        laptop.sync()
        val before = laptop.cursor()
        for (i in 1..3) desk.put("category", "c$i", cat("C$i"))
        desk.sync()

        // The disk refuses the records from now on: a directory where their file goes stands in for a full disk.
        val records = File(dir, "account-records.json")
        records.delete()
        File(records, "x").mkdirs()
        laptop.sync()
        assertTrue("the pull moved the cursor in memory", laptop.cursor() > before)
        val offline = AccountHttp { _, _, _, _, _, _ -> throw AccountError("network") }
        assertEquals("but not on disk", before, device(srv, "fresh", dir = dir, http = offline).cursor())

        // Once the disk takes them again, the next save catches up.
        records.deleteRecursively()
        laptop.sync()
        val again = device(srv, "again", dir = dir, http = offline)
        assertEquals(laptop.cursor(), again.cursor())
        assertEquals(3, again.list("category").size)
    }

    @Test
    fun `the export holds every record, readable`() {
        val srv = server()
        val desk = device(srv)
        desk.put("category", "money", cat("Money"))
        val data = desk.exportData()
        assertEquals(kotlinx.serialization.json.JsonNull, data["email"])
        assertEquals(
            """[{"collection":"category","key":"money","value":{"name":"Money"},"changed":"${AccountService.iso(desk.list("category").single().t)}"}]""",
            data["records"].toString(),
        )
    }

    @Test
    fun `the export leaves out the booking key and another account's booking records, and the next account never gets them`() = runBlocking {
        val srv = server()
        val phone = device(srv)
        phone.signUp("one@example.com", PASSWORD)
        val one = AccountService.bookingAcct("one@example.com")
        assertEquals(16, one.length)
        // What desktop's booking service writes, as it comes down with the rest of the account.
        phone.put("bookingkey", one, buildJsonObject { put("current", "private") })
        phone.put("booking", "settings:$one", buildJsonObject { put("url", "https://rukoo.io/one") })
        phone.put("booking", "map:$one:abc", buildJsonObject { put("eventId", "e1") })
        phone.put("category", "money", cat("Money"))
        phone.sync()
        fun exported() = (phone.exportData()["records"] as JsonArray).map { r ->
            val o = r as JsonObject
            (o["collection"] as JsonPrimitive).content to (o["key"] as JsonPrimitive).content
        }.toSet()
        assertEquals(setOf("booking" to "settings:$one", "booking" to "map:$one:abc", "category" to "money"), exported())

        // Signed out: the key leaves with the session; the booking records stay here, but out of the export.
        phone.signOut()
        assertNull(phone.value("bookingkey", one))
        assertNotNull(phone.value("booking", "settings:$one"))
        assertEquals(setOf("category" to "money"), exported())

        // Another account on the same phone: only the category goes up, and its export has no booking records.
        phone.signUp("two@example.com", PASSWORD)
        phone.sync()
        assertEquals(1, srv.accounts.getValue("two@example.com").records.values.count { it.data != null })
        assertEquals(setOf("category" to "money"), exported())

        // The first account again: its key comes back from the account.
        phone.signOut()
        phone.signIn("one@example.com", PASSWORD)
        phone.sync()
        assertEquals(buildJsonObject { put("current", "private") }, phone.value("bookingkey", one))
        assertEquals(setOf("booking" to "settings:$one", "booking" to "map:$one:abc", "category" to "money"), exported())
    }

    @Test
    fun `without a server the account says so`() = runBlocking {
        val desk = device(null)
        rejects("no_server") { desk.signUp("o@example.com", PASSWORD) }
        try {
            desk.setServer("http://example.com")
            fail("http elsewhere is refused")
        } catch (e: AccountError) {
            assertEquals("no_server", e.code)
        }
        desk.setServer("https://sync.example.com")
        assertEquals("https://sync.example.com", desk.status().server)
    }

    @Test
    fun `the server can only change while signed out`() = runBlocking {
        val srv = server()
        val desk = device(srv)
        desk.signUp("p@example.com", PASSWORD)
        try {
            desk.setServer("https://elsewhere.example.com")
            fail("changed the server while signed in")
        } catch (e: AccountError) {
            assertEquals("signed_in", e.code)
        }
        desk.signOut()
        desk.setServer("https://elsewhere.example.com")
        assertEquals("https://elsewhere.example.com", desk.status().server)
        // An empty address goes back to the default.
        desk.setServer("")
        assertEquals(srv.url, desk.status().server)
    }

    @Test
    fun `a change goes up by itself shortly after it is made`() = runBlocking {
        val srv = server()
        val desk = device(srv, pushAfterMs = 50)
        desk.signUp("q@example.com", PASSWORD)
        desk.sync()
        desk.put("sender", "x@example.com", rule("money"))
        withTimeout(10_000) {
            while (srv.accounts.getValue("q@example.com").records.isEmpty() || desk.status().pending > 0) delay(20)
        }
        assertEquals(0, desk.status().pending)
    }

    @Test
    fun `the server's info is kept for the server it came from`() = runBlocking {
        val srv = server()
        val desk = device(srv)
        val info = desk.serverInfo()
        assertEquals("Test operator", info.operator?.name)
        assertEquals("eu", info.hosting?.jurisdiction)
        assertEquals(info, desk.status().info)
        desk.setServer("https://elsewhere.example.com")
        assertNull("another server, another operator", desk.status().info)
    }

    private companion object {
        const val PASSWORD = "a long enough password"

        // Rukoo's former default address and today's.
        const val OLD_DEFAULT = "https://rukoo-account.bvdm.ai"
        const val NEW_DEFAULT = "https://rukoo.io"
    }
}
