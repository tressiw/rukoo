package nl.bvdm.rukoo.account

import java.io.File
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.runBlocking
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put
import nl.bvdm.rukoo.core.RukooJson
import okhttp3.OkHttpClient
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Assume.assumeTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder

/**
 * The phone's account code against a real account server: the Worker in server/ under `wrangler dev`, or a deployed
 * one. Set RUKOO_ACCOUNT_TEST_SERVER to its address (http://127.0.0.1:8790 for `npx wrangler dev --port 8790`);
 * without it these tests skip. They make throwaway accounts and delete them again. The server must allow sign-ups
 * ("open"), or take the invite code in RUKOO_ACCOUNT_TEST_INVITE.
 *
 * With RUKOO_ACCOUNT_TEST_NODE set to a Node.js binary as well, the last test also runs desktop Rukoo's account code
 * (src/main/account) in that Node, so a desktop and a phone share one account. Desktop's dependencies must be
 * installed (npm install at the repository root, or NODE_PATH pointing at a node_modules that has them).
 */
class AccountLiveTest {
    @get:Rule
    val tmp = TemporaryFolder()

    private val server = System.getenv("RUKOO_ACCOUNT_TEST_SERVER").orEmpty()
    private val invite = System.getenv("RUKOO_ACCOUNT_TEST_INVITE").orEmpty()
    private val node = System.getenv("RUKOO_ACCOUNT_TEST_NODE").orEmpty()
    private val scopes = mutableListOf<CoroutineScope>()
    private val devices = mutableListOf<AccountService>()
    private val okhttp = OkHttpClient()

    @After
    fun tearDown() {
        devices.forEach { it.close() }
        scopes.forEach { it.cancel() }
    }

    private fun live() = assumeTrue("set RUKOO_ACCOUNT_TEST_SERVER to run against a real account server", server.isNotEmpty())

    private fun email() = "test-${AccountKeys.b64u(AccountKeys.randomBytes(6)).lowercase().replace(Regex("[^a-z0-9]"), "x")}@example.com"

    private fun device(name: String): AccountService {
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default).also { scopes += it }
        return AccountService(tmp.newFolder(), PlainSecrets, server, name, OkHttpAccountHttp(okhttp), scope, devHosts = true, pushAfterMs = 60_000)
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

    @Test
    fun `the server describes itself`() = runBlocking<Unit> {
        live()
        val info = device("phone").serverInfo()
        assertEquals(1, info.protocol)
        assertTrue(info.signups in setOf("open", "invite", "closed"))
    }

    @Test
    fun `two phones share records, conflicts settle, and the account goes away`() = runBlocking<Unit> {
        live()
        val address = email()
        val phone = device("phone")
        phone.put("category", "needs-you", buildJsonObject {
            put("name", "Needs you")
            put("order", 0)
        })
        val recoveryKey = phone.signUp(address, PASSWORD, invite).recoveryKey
        phone.sync()

        val tablet = device("tablet")
        tablet.put("sender", "shop@example.com", rule("promotions"))
        tablet.signIn(address.uppercase(), PASSWORD)
        tablet.sync()
        assertEquals(buildJsonObject {
            put("name", "Needs you")
            put("order", 0)
        }, tablet.get("category", "needs-you"))
        phone.sync()
        assertEquals(rule("promotions"), phone.get("sender", "shop@example.com"))

        // The first classification stays; the newest sender rule wins; a deletion travels.
        val key = "$address\n<m1@example.com>"
        phone.put("classification", key, rule("money"), ifAbsent = true)
        tablet.put("classification", key, rule("updates"), ifAbsent = true)
        phone.sync()
        tablet.sync()
        assertEquals(rule("money"), tablet.get("classification", key))
        tablet.put("sender", "shop@example.com", rule("to-read"))
        tablet.sync()
        phone.sync()
        assertEquals(rule("to-read"), phone.get("sender", "shop@example.com"))
        phone.remove("category", "needs-you")
        phone.sync()
        tablet.sync()
        assertNull(tablet.get("category", "needs-you"))

        // Many records, in pages both ways.
        for (i in 0 until 1200) phone.put("classification", "$address\n<bulk-$i@example.com>", rule("updates"), ifAbsent = true)
        phone.sync()
        tablet.sync()
        assertEquals(1201, tablet.list("classification").size)

        val details = phone.details()
        assertEquals(listOf("phone", "tablet"), details.devices.map { it.name }.sorted())
        // A deletion goes up sealed with d: true, so the server counts it like any record: it can't tell them apart.
        // (Desktop's test/account-live.test.js still expects 1202 from before deletions were sealed.)
        assertEquals("a sender rule, 1201 classifications and the deleted category", 1203L, details.usage?.records)

        // A wrong password on a signed-in call is a typo, not an ended session.
        rejects("wrong_password") { phone.changePassword("not the password!", "another long password") }
        assertTrue(phone.status().signedIn)
        phone.changePassword(PASSWORD, "another long password")
        rejects("signed_out") { tablet.sync() }

        // The recovery key brings the account back under a new password, with the data.
        val other = device("other")
        other.recover(address, recoveryKey, "a recovered password")
        other.sync()
        assertEquals(rule("to-read"), other.get("sender", "shop@example.com"))
        rejects("signed_out") { phone.sync() }

        other.deleteAccount("a recovered password")
        rejects("unauthorized") { device("again").signIn(address, "a recovered password") }
    }

    /** Desktop's AccountService in Node: signs up or in, writes what [write] says, syncs, and prints every record. */
    private fun desktop(step: String, address: String, password: String, write: JsonObject = JsonObject(emptyMap())): JsonObject {
        val repo = listOf(File("../.."), File("..")).first { File(it, "src/main/account/index.js").exists() }.canonicalFile
        val script = """
            const path = require('path'), fs = require('fs'), os = require('os');
            const [repo, server, step, email, password] = process.argv.slice(1);
            const write = process.env.RUKOO_DESKTOP_WRITE;
            const { AccountService } = require(path.join(repo, 'src/main/account'));
            const secrets = { encrypt: (s) => 'plain:' + Buffer.from(s).toString('base64'), decrypt: (s) => Buffer.from(String(s).slice(6), 'base64').toString() };
            (async () => {
              const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'rukoo-desktop-'));
              const acc = new AccountService({ dataDir: dir, secrets, defaultServer: server, deviceName: 'desktop', platform: 'linux' }).init();
              try {
                const w = JSON.parse(write);
                if (step === 'signup') await acc.signUp({ email, password });
                else await acc.signIn({ email, password });
                await acc.sync();
                for (const r of w.put || []) acc.put(r.c, r.k, r.v, { ifAbsent: Boolean(r.ifAbsent) });
                for (const r of w.remove || []) acc.remove(r.c, r.k);
                await acc.sync();
                const out = {};
                for (const c of ['category', 'classification', 'override', 'sender', 'setting']) out[c] = acc.list(c).map(({ k, v }) => ({ k, v }));
                process.stdout.write(JSON.stringify(out));
              } finally {
                acc.close();
                fs.rmSync(dir, { recursive: true, force: true });
              }
            })().catch((e) => { process.stderr.write(String(e.code || e.message)); process.exit(1); });
        """.trimIndent()
        // What to write goes through the environment: on Windows, an argument loses its double quotes on the way to Node.
        val process = ProcessBuilder(node, "-e", script, repo.path, server, step, address, password)
            .redirectErrorStream(false)
            .apply { environment()["RUKOO_DESKTOP_WRITE"] = write.toString() }
            .start()
        val out = process.inputStream.bufferedReader().readText()
        val err = process.errorStream.bufferedReader().readText()
        assertTrue("desktop $step timed out", process.waitFor(120, TimeUnit.SECONDS))
        assertEquals("desktop $step failed: $err", 0, process.exitValue())
        return RukooJson.parseToJsonElement(out).jsonObject
    }

    private fun JsonObject.value(c: String, k: String): JsonElement? =
        (this[c] as? JsonArray)?.map { it.jsonObject }?.firstOrNull { it["k"]?.jsonPrimitive?.content == k }?.get("v")

    @Test
    fun `desktop and the phone read each other's records`() = runBlocking<Unit> {
        live()
        assumeTrue("set RUKOO_ACCOUNT_TEST_NODE to a Node.js binary to run desktop's account code", node.isNotEmpty())
        val address = email()
        val m1 = "$address\n<m1@example.com>"
        val m2 = "$address\n<m2@example.com>"
        val m3 = "$address\n<m3@example.com>"
        fun quoted(text: String) = kotlinx.serialization.json.JsonPrimitive(text).toString()
        val categories = buildJsonObject {
            put("enabled", true)
            put("unsure", "to-read")
            put("seeded", true)
        }

        // Desktop makes the account and writes first.
        val first = desktop(
            "signup",
            address,
            PASSWORD,
            RukooJson.parseToJsonElement(
                """{"put":[
                    {"c":"category","k":"needs-you","v":{"name":null,"description":null,"color":"#ff5d73","order":0}},
                    {"c":"category","k":"c-kids","v":{"name":"Kids","description":"School","color":"#4a7dff","order":5}},
                    {"c":"sender","k":"boss@example.com","v":{"category":"needs-you","at":1}},
                    {"c":"classification","k":${quoted(m1)},"v":{"category":"money","source":"rules","at":1},"ifAbsent":true},
                    {"c":"classification","k":${quoted(m3)},"v":{"category":"needs-you","source":"rules","at":1},"ifAbsent":true},
                    {"c":"setting","k":"categories","v":$categories}
                ]}""",
            ).jsonObject,
        )
        assertEquals(rule("money").getValue("category"), first.value("classification", m1)?.jsonObject?.get("category"))

        // The phone signs in and finds all of it. It had sorted m3 itself before; desktop's first write wins.
        val phone = device("phone")
        phone.put("classification", m3, rule("updates"), ifAbsent = true)
        phone.signIn(address, PASSWORD)
        phone.sync()
        assertEquals("#ff5d73", phone.get("category", "needs-you")?.jsonObject?.get("color")?.jsonPrimitive?.content)
        assertEquals("Kids", phone.get("category", "c-kids")?.jsonObject?.get("name")?.jsonPrimitive?.content)
        assertEquals("needs-you", phone.get("sender", "boss@example.com")?.jsonObject?.get("category")?.jsonPrimitive?.content)
        assertEquals("money", phone.get("classification", m1)?.jsonObject?.get("category")?.jsonPrimitive?.content)
        assertEquals(categories, phone.get("setting", "categories"))
        assertEquals("needs-you", phone.get("classification", m3)?.jsonObject?.get("category")?.jsonPrimitive?.content)

        // The phone writes: a correction, a newer sender rule, a deleted category, and a first write that loses to desktop's.
        phone.put("override", m1, buildJsonObject {
            put("category", "updates")
            put("at", 2)
        })
        phone.put("sender", "boss@example.com", buildJsonObject {
            put("category", "to-read")
            put("at", 2)
        })
        phone.remove("category", "c-kids")
        phone.put("classification", m2, buildJsonObject {
            put("category", "promotions")
            put("source", "jev")
            put("confidence", 0.82)
            put("model", "jev-1.13.0")
            put("at", 2)
        }, ifAbsent = true)
        phone.sync()

        // Desktop signs in on a fresh computer and reads what the phone wrote.
        val second = desktop("signin", address, PASSWORD)
        assertEquals("updates", second.value("override", m1)?.jsonObject?.get("category")?.jsonPrimitive?.content)
        assertEquals("to-read", second.value("sender", "boss@example.com")?.jsonObject?.get("category")?.jsonPrimitive?.content)
        assertNull("the phone's deletion reached desktop", second.value("category", "c-kids"))
        assertEquals("0.82", second.value("classification", m2)?.jsonObject?.get("confidence")?.jsonPrimitive?.content)
        assertEquals("desktop's first write stays", "needs-you", second.value("classification", m3)?.jsonObject?.get("category")?.jsonPrimitive?.content)
        assertFalse(phone.status().pending > 0)

        phone.deleteAccount(PASSWORD)
    }

    private companion object {
        const val PASSWORD = "a long enough password"
    }
}
