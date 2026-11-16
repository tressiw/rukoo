package nl.bvdm.rukoo.mcp

import java.net.InetAddress
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import nl.bvdm.rukoo.agents.AgentSettings
import nl.bvdm.rukoo.core.RukooJson
import nl.bvdm.rukoo.core.Tailnet
import nl.bvdm.rukoo.mcp.tools.McpTools

/**
 * Which connections count as this phone: its own Tailscale address, plus loopback in debug builds for `adb forward`
 * during development. The Host names are computed per request from the current address, so a new Tailscale address
 * works without a restart. [advertised] (debug builds only) is a `host:port` other devices reach an emulator at.
 */
class TailnetPolicy(
    private val debug: Boolean,
    private val ownAddress: () -> InetAddress? = { Tailnet.ownAddress() },
    private val isOwn: (InetAddress) -> Boolean = Tailnet::isOwnAddress,
    private val advertised: () -> String? = { null },
) : EndpointPolicy {
    override fun accepts(local: InetAddress): Boolean = isOwn(local) || (debug && local.isLoopbackAddress)

    override fun hosts(local: InetAddress, port: Int): Collection<String> = buildSet {
        ownAddress()?.hostAddress?.let { add("$it:$port") }
        if (isOwn(local) && local is java.net.Inet4Address) add("${local.hostAddress}:$port")
        if (debug) {
            add("127.0.0.1:$port")
            add("localhost:$port")
            advertised()?.let { add(it.lowercase()) }
        }
    }

    override fun endpoint(local: InetAddress, localPort: Int, host: String): String {
        if (debug && local.isLoopbackAddress) {
            // A relay to the emulator: the bridge connected to the advertised address and proves against that one.
            val adv = advertised()
            if (adv != null && host.equals(adv, ignoreCase = true)) return adv
            return "127.0.0.1:$localPort"
        }
        return "${local.hostAddress.orEmpty().removePrefix("::ffff:")}:$localPort"
    }
}

/** The server and everything it answers with, started and stopped as one by [RukooMcpService]. */
class McpRuntime(
    port: Int,
    policy: EndpointPolicy,
    secrets: () -> AuthSecrets,
    tools: McpTools,
    version: String,
    deviceName: () -> String,
    log: (String, Throwable?) -> Unit = { _, _ -> },
    onCaller: (McpCaller) -> Unit = {},
) {
    val auth = McpAuth(secrets)
    val handler = McpHandler(tools, version, deviceName, log)
    val server = McpHttpServer(port, policy, auth, handler, log, onCaller = onCaller)

    /** Throws an IOException when the port is taken. */
    fun start() = server.start()

    fun stop() = server.stop()
}

/** The texts Settings offers for connecting agents to this phone (spec 2 §3.7-3.8). */
object McpSetup {
    /** The secrets the server accepts tokens from, as the settings are now. */
    fun secrets(toolsOn: Boolean, agents: AgentSettings): AuthSecrets = AuthSecrets(
        toolsOn = toolsOn,
        clarkKey = agents.clark.key,
        openClawToken = agents.openclaw.token,
        openClawRemote = agents.openclaw.remote,
        computers = agents.computers.filter { it.mcpToken.isNotEmpty() }.map { it.pairingId to it.mcpToken },
    )

    /** The variable OpenClaw's MCP entry reads Rukoo's token from: desktop's name, as OpenClaw holds one Rukoo. */
    const val OPENCLAW_TOKEN_VAR = "RUKOO_MCP_TOKEN"

    /**
     * desktop openclawSetup (config.js): the two steps that point OpenClaw's MCP config at this phone. Step 1 is a
     * line for the gateway's .env file with the token; step 2 is the command, whose header names the variable, so
     * the token stays out of openclaw.json. The comment lines hold no shell syntax and the JSON is single-quoted, so
     * pasting it all into a shell does no harm. OpenClaw names one Rukoo, so it replaces a computer's entry.
     */
    fun openClaw(url: String, gatewayToken: String): String? {
        val token = McpTokens.openClawRemote(gatewayToken)
        if (token.isEmpty()) return null
        val server = buildJsonObject {
            put("url", url)
            put("transport", "streamable-http")
            put("headers", buildJsonObject { put("Authorization", "Bearer \${$OPENCLAW_TOKEN_VAR}") })
        }
        return listOf(
            "# Step 1, not a command: put this line in the OpenClaw .env file, ~/.openclaw/.env or \$OPENCLAW_STATE_DIR/.env, in place of any older $OPENCLAW_TOKEN_VAR line.",
            "$OPENCLAW_TOKEN_VAR=$token",
            "# Step 2: run this command where the gateway runs, then restart the gateway.",
            "openclaw mcp set rukoo '${RukooJson.encodeToString(JsonElement.serializer(), server)}'",
            "",
        ).joinToString("\n")
    }
}
