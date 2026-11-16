package nl.bvdm.rukoo.mcp

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class McpAuthTest {
    private fun resource(name: String) = javaClass.classLoader!!.getResource(name)!!.readText()

    // Made with the real bridge code (rukoo_bridge.py): the token from RUKOO_KEY and the proof it expects.
    @Test
    fun matchesTheBridgeVectors() {
        val vectors = Json.parseToJsonElement(resource("mcp/proof-vectors.json")) as JsonArray
        for (v in vectors) {
            val o = v.jsonObject
            val key = o["key"]!!.jsonPrimitive.content
            val token = McpTokens.clark(key)
            assertEquals(o["token"]!!.jsonPrimitive.content, token)
            assertEquals(o["proof"]!!.jsonPrimitive.content, McpTokens.proof(token, o["endpoint"]!!.jsonPrimitive.content, o["challenge"]!!.jsonPrimitive.content))
        }
    }

    // spec 2 §3.2, computed with desktop's config.js.
    @Test
    fun matchesDesktopVectors() {
        assertEquals("quaOZuQV6-6XJ0ZlwrKJ7aFrxAYfDMaNiqzSCMVS-7I", McpTokens.clark("hermes-key"))
        assertEquals("46-_kZiRjKr4CiPVZzCrZ5iFgKjWOfZN8z8g1vehUQM", McpTokens.proof(McpTokens.clark("hermes-key"), "100.64.0.3:47800", "challenge-0123456789"))
        assertEquals("uE_hQuFxFtxWG4T8oU5PsTUz3Q6GPB03jYtDiXrdiSw", McpTokens.openClawRemote("gw-token"))
        assertEquals("", McpTokens.clark(""))
        // Rukoo trims the key on save and the bridge trims RUKOO_KEY.
        assertEquals(McpTokens.clark("hermes-key"), McpTokens.clark("  hermes-key\n"))
    }

    @Test
    fun identifiesEachCallerOnlyWhileAllowed() {
        var secrets = AuthSecrets(
            toolsOn = true, clarkKey = "hermes-key", openClawToken = "gw-token", openClawRemote = true,
            computers = listOf("ph_a" to "computer-token-a", "ph_b" to "computer-token-b"),
        )
        val auth = McpAuth { secrets }
        assertEquals(McpCaller.Clark, auth.identify(McpTokens.clark("hermes-key")))
        assertEquals(McpCaller.OpenClaw, auth.identify(McpTokens.openClawRemote("gw-token")))
        assertEquals(McpCaller.Computer("ph_b"), auth.identify("computer-token-b"))
        assertNull(auth.identify("wrong"))
        assertNull(auth.identify(""))
        assertNull(auth.identify("x".repeat(513)))
        // The raw secrets are not tokens.
        assertNull(auth.identify("hermes-key"))
        assertNull(auth.identify("gw-token"))
        // The local OpenClaw token (desktop's loopback listener) is not accepted on the phone.
        assertNull(auth.identify("R3_AgRARLvbWP1F9uK6ZMZzDZVKbl0YqXe0uKo0JdCA"))

        secrets = secrets.copy(openClawRemote = false)
        assertNull(auth.identify(McpTokens.openClawRemote("gw-token")))
        secrets = secrets.copy(toolsOn = false)
        assertNull(auth.identify(McpTokens.clark("hermes-key")))
        assertNull(auth.identify("computer-token-a"))
    }

    @Test
    fun provesOnlyWithAKeyAndToolsOn() {
        var secrets = AuthSecrets(toolsOn = true, clarkKey = "hermes-key")
        val auth = McpAuth { secrets }
        assertEquals("46-_kZiRjKr4CiPVZzCrZ5iFgKjWOfZN8z8g1vehUQM", auth.proof("challenge-0123456789", "100.64.0.3:47800"))
        secrets = secrets.copy(toolsOn = false)
        assertEquals("", auth.proof("challenge-0123456789", "100.64.0.3:47800"))
        secrets = AuthSecrets(toolsOn = true)
        assertEquals("", auth.proof("challenge-0123456789", "100.64.0.3:47800"))
    }

    @Test
    fun buildsTheOpenClawSetup() {
        val setup = McpSetup.openClaw("http://100.64.0.3:47800/mcp", "gw-token")!!
        val lines = setup.lines()
        // Step 1 is the only line with the token, for the gateway's .env file; the command names the variable.
        assertEquals("RUKOO_MCP_TOKEN=uE_hQuFxFtxWG4T8oU5PsTUz3Q6GPB03jYtDiXrdiSw", lines[1])
        assertEquals(listOf(lines[1]), lines.filter { "uE_hQuFxFtxWG4T8oU5PsTUz3Q6GPB03jYtDiXrdiSw" in it })
        assertEquals(
            "openclaw mcp set rukoo '{\"url\":\"http://100.64.0.3:47800/mcp\",\"transport\":\"streamable-http\",\"headers\":{\"Authorization\":\"Bearer \${RUKOO_MCP_TOKEN}\"}}'",
            lines[3],
        )
        assertTrue(lines[0].startsWith("# Step 1") && lines[2].startsWith("# Step 2"))
        // The labels hold nothing a shell would act on when the whole text is pasted into one.
        for (label in listOf(lines[0], lines[2])) assertFalse(label, label.any { it in "'\"`()[]{};&|<>*?!\\" })
        assertNull(McpSetup.openClaw("http://100.64.0.3:47800/mcp", ""))
    }
}
