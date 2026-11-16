package nl.bvdm.rukoo.ui.settings

import android.annotation.SuppressLint
import android.content.Context
import android.content.Intent
import android.os.Build
import android.os.PowerManager
import android.provider.Settings
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import androidx.core.net.toUri
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.repeatOnLifecycle
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import nl.bvdm.rukoo.BuildConfig
import nl.bvdm.rukoo.R
import nl.bvdm.rukoo.core.Tailnet
import nl.bvdm.rukoo.mcp.AndroidMcpController
import nl.bvdm.rukoo.mcp.McpDebug
import nl.bvdm.rukoo.mcp.McpStatus
import nl.bvdm.rukoo.mcp.McpTexts
import nl.bvdm.rukoo.settings.AppSettings
import nl.bvdm.rukoo.ui.components.DotTone
import nl.bvdm.rukoo.ui.components.LocalRkToast
import nl.bvdm.rukoo.ui.components.RkLinkButton
import nl.bvdm.rukoo.ui.components.RkTextField
import nl.bvdm.rukoo.ui.components.SectionTitle
import nl.bvdm.rukoo.ui.components.SettingsCard
import nl.bvdm.rukoo.ui.components.SettingsRow
import nl.bvdm.rukoo.ui.components.SettingsSwitchRow
import nl.bvdm.rukoo.ui.components.StatusDot
import nl.bvdm.rukoo.ui.mail.rememberMailDates
import nl.bvdm.rukoo.ui.icons.PanelIcons
import nl.bvdm.rukoo.ui.root.AppViewModel

/**
 * Phone tools: the MCP server on this phone's Tailscale address that lets agents read the mail here and
 * put drafts on its screen. Its switch, port and status, who can connect, OpenClaw's setup, and the battery
 * setting Samsung needs. The words for the server's state are the server's own ([McpTexts]).
 */
@Composable
fun PhoneToolsPage(app: AppViewModel) {
    val s = app.services
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val toast = LocalRkToast.current
    val settings by s.settingsSnapshot.collectAsStateWithLifecycle()
    val agentSettings by s.agentSettings.settings.collectAsStateWithLifecycle()
    val status by s.mcp.status.collectAsStateWithLifecycle()
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    var tailnet by remember { mutableStateOf<String?>(null) }
    var unrestricted by remember { mutableStateOf(true) }
    var portDialog by remember { mutableStateOf(false) }
    LaunchedEffect(lifecycle) {
        lifecycle.repeatOnLifecycle(Lifecycle.State.RESUMED) {
            unrestricted = isUnrestricted(context)
            // Tailscale can connect while this page is open.
            while (true) {
                tailnet = withContext(Dispatchers.IO) { Tailnet.ownAddress()?.hostAddress }
                delay(3_000)
            }
        }
    }
    val clark = agentSettings.clark
    val openclaw = agentSettings.openclaw
    val port = if (status.running && status.port > 0) status.port else settings.mcpPort
    // The note reads the stored settings and the address; it follows both.
    val hermesNote = remember(status, clark, tailnet) { s.mcp.hermesNote() }

    SettingsPageBody {
        SettingsIntro(stringResource(R.string.settings_phone_tools_intro))
        Spacer(Modifier.height(12.dp))
        SettingsCard {
            SettingsSwitchRow(
                title = stringResource(R.string.settings_phone_tools_toggle),
                description = stringResource(R.string.settings_phone_tools_toggle_help),
                checked = settings.mcpEnabled,
                onCheckedChange = { on ->
                    scope.launch {
                        s.settings.update { it.copy(mcpEnabled = on) }
                        if (on) s.mcp.start() else s.mcp.stop()
                    }
                },
            )
            if (settings.mcpEnabled) {
                val shown = PhoneToolsTexts.status(status)
                val label = when (shown) {
                    PhoneToolsTexts.Shown.Reachable -> stringResource(R.string.settings_phone_tools_reachable, status.url.orEmpty())
                    PhoneToolsTexts.Shown.Starting -> stringResource(R.string.settings_phone_tools_starting)
                    PhoneToolsTexts.Shown.Problem -> McpTexts.status(context, status).orEmpty()
                }
                val agents = McpTexts.agents(context, status)
                // Who may connect, in words; a start that failed keeps its technical cause in small mono type.
                val note = when {
                    shown != PhoneToolsTexts.Shown.Reachable -> null
                    agents.isEmpty() -> stringResource(R.string.settings_phone_tools_no_agents)
                    else -> stringResource(R.string.settings_phone_tools_for, agents.joinToString(", "))
                }
                StatusLine(
                    label,
                    shown.tone,
                    Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 14.dp),
                    detail = status.detail?.takeIf { shown == PhoneToolsTexts.Shown.Problem && status.error == McpStatus.ERROR_FAILED },
                    pulsing = shown.tone == DotTone.BUSY,
                    note = note,
                )
            }
            SettingsRow(
                title = stringResource(R.string.settings_phone_tools_port),
                description = stringResource(R.string.settings_phone_tools_port_help),
                value = port.toString(),
                onClick = { portDialog = true },
            )
        }

        SectionTitle(stringResource(R.string.settings_phone_tools_who))
        SettingsCard {
            val clarkName = clark.name.ifBlank { "Hermes" }
            // The server lets Clark in with its key alone (McpSetup.secrets), as desktop's hasKey: the bridge on
            // Clark's machine finds the phone itself, so its URL here isn't needed.
            val clarkOn = clark.key.isNotBlank()
            // Whether the bridge got here yet tells an outdated bridge (which never looks for a phone) from a
            // working one, so the setup note stays until it has.
            val seen = status.clarkSeen?.takeIf { settings.mcpEnabled && clarkOn }
            val dates = rememberMailDates()
            val description = when {
                !clarkOn -> stringResource(R.string.settings_phone_tools_clark_off, clarkName)
                seen != null -> stringResource(
                    R.string.settings_phone_tools_clark_seen,
                    if (dates.sameDay(seen, System.currentTimeMillis())) dates.time(seen) else "${dates.numericDate(seen)} ${dates.time(seen)}",
                )
                !settings.mcpEnabled -> hermesNote.ifBlank { stringResource(R.string.settings_phone_tools_clark_help, clarkName) }
                else -> stringResource(R.string.settings_phone_tools_clark_not_seen) + " " + hermesNote.ifBlank { stringResource(R.string.settings_phone_tools_clark_help, clarkName) }
            }
            SettingsRow(
                title = stringResource(R.string.settings_phone_tools_clark, clarkName),
                description = description,
                leading = { StatusDot(if (seen != null) DotTone.READY else DotTone.MUTED, size = 8.dp) },
            )
            for (c in agentSettings.computers) {
                SettingsRow(
                    title = stringResource(R.string.settings_phone_tools_computer, c.name),
                    description = stringResource(if (settings.mcpEnabled) R.string.settings_phone_tools_computer_help else R.string.settings_phone_tools_computer_off),
                    leading = { StatusDot(if (settings.mcpEnabled) DotTone.READY else DotTone.MUTED, size = 8.dp) },
                )
            }
            // A gateway elsewhere signs in with the token Rukoo derives from the gateway token; its URL isn't needed.
            val gatewayToken = openclaw.token.isNotBlank()
            SettingsSwitchRow(
                title = stringResource(R.string.settings_phone_tools_openclaw),
                description = stringResource(if (gatewayToken) R.string.settings_phone_tools_openclaw_help else R.string.settings_phone_tools_openclaw_needs_token),
                checked = openclaw.remote && gatewayToken,
                enabled = gatewayToken,
                onCheckedChange = { on -> scope.launch { s.agentSettings.update { it.copy(openclaw = it.openclaw.copy(remote = on)) } } },
            )
            val needToken = stringResource(R.string.settings_agents_token_first)
            val noAddress = stringResource(R.string.settings_phone_tools_no_tailnet)
            val copyLabel = stringResource(R.string.settings_agents_copy_open_claw)
            SettingsRow(
                title = copyLabel,
                description = stringResource(R.string.settings_phone_tools_copy_open_claw_help),
                leading = { RowIcon(PanelIcons.Copy) },
                enabled = gatewayToken,
                onClick = {
                    // The server words the two steps (with the token): one setup for Settings and the debug tools alike.
                    val command = if (gatewayToken) s.mcp.openClawSetup() else null
                    when {
                        !gatewayToken -> toast.showError(needToken)
                        command == null -> toast.showError(noAddress)
                        else -> {
                            copySensitive(context, copyLabel, command)
                            val url = PhoneToolsTexts.setupUrl(command).orEmpty()
                            toast.showToast(context.getString(R.string.settings_phone_tools_copied_open_claw, url), durationMs = 8000L)
                        }
                    }
                },
            )
        }

        SectionTitle(stringResource(R.string.settings_phone_tools_battery))
        SettingsCard {
            SettingsRow(
                title = stringResource(R.string.settings_phone_tools_battery_row),
                description = stringResource(if (isSamsung()) R.string.settings_phone_tools_battery_help else R.string.settings_phone_tools_battery_help_other),
                value = stringResource(if (unrestricted) R.string.settings_phone_tools_battery_unrestricted else R.string.settings_phone_tools_battery_optimized),
                leading = { RowIcon(PanelIcons.Bolt) },
                onClick = { openBatterySettings(context) },
            )
        }

        if (BuildConfig.DEBUG) DebugAddress(app, status)
    }

    if (portDialog) {
        PortDialog(
            port = settings.mcpPort,
            onSave = { next -> scope.launch { s.settings.update { it.copy(mcpPort = next) } } },
            onDismiss = { portDialog = false },
        )
    }
}

/** Choosing the port; the server moves to it at once (AndroidMcpController follows the setting). */
@Composable
private fun PortDialog(port: Int, onSave: (Int) -> Unit, onDismiss: () -> Unit) {
    var text by remember { mutableStateOf(port.toString()) }
    val parsed = PhoneToolsTexts.port(text)
    RkDialog(
        title = stringResource(R.string.settings_phone_tools_port),
        confirmLabel = stringResource(R.string.common_actions_save),
        onConfirm = {
            parsed?.let(onSave)
            onDismiss()
        },
        onDismiss = onDismiss,
        confirmEnabled = parsed != null,
    ) {
        RkTextField(
            value = text,
            onValueChange = { text = it.filter(Char::isDigit).take(5) },
            label = stringResource(R.string.settings_phone_tools_port),
            placeholder = AppSettings.DEFAULT_MCP_PORT.toString(),
            error = if (parsed == null && text.isNotEmpty()) stringResource(R.string.settings_phone_tools_port_invalid) else null,
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
        )
    }
}

/**
 * Debug builds only: the address other devices reach this phone at instead of its Tailscale address
 * ([McpDebug]), so an emulator behind `adb forward` and a relay can be tested end to end.
 */
@Composable
private fun DebugAddress(app: AppViewModel, status: McpStatus) {
    val context = LocalContext.current
    val toast = LocalRkToast.current
    var dialog by remember { mutableStateOf(false) }
    var current by remember { mutableStateOf(McpDebug.advertised(context)) }
    val invalid = stringResource(R.string.settings_phone_tools_debug_invalid)
    SectionTitle(stringResource(R.string.settings_phone_tools_debug))
    SettingsCard {
        SettingsRow(
            title = stringResource(R.string.settings_phone_tools_debug_address),
            description = stringResource(R.string.settings_phone_tools_debug_address_help),
            value = current ?: stringResource(R.string.settings_off),
            leading = { RowIcon(PanelIcons.Wrench) },
            onClick = { dialog = true },
        )
    }
    if (dialog) {
        var text by remember { mutableStateOf(current ?: status.advertised.orEmpty()) }
        fun apply(value: String?) {
            // McpDebug drops an address it can't use; the one saved before stays then.
            val before = McpDebug.advertised(context)
            McpDebug.setAdvertised(context, value)
            val stored = McpDebug.advertised(context)
            if (!value.isNullOrBlank() && stored == null) {
                McpDebug.setAdvertised(context, before)
                toast.showError(invalid)
                return
            }
            current = stored
            // A new address needs a new socket, as the debug receiver does.
            (app.services.mcp as? AndroidMcpController)?.restart()
            dialog = false
        }
        RkDialog(
            title = stringResource(R.string.settings_phone_tools_debug_address),
            confirmLabel = stringResource(R.string.common_actions_save),
            onConfirm = { apply(text.trim()) },
            onDismiss = { dialog = false },
        ) {
            RkTextField(
                value = text,
                onValueChange = { text = it },
                label = stringResource(R.string.settings_phone_tools_debug_address),
                placeholder = "100.64.0.9:47800",
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri, autoCorrectEnabled = false),
            )
            RkLinkButton(stringResource(R.string.common_actions_clear), { apply(null) }, Modifier.padding(top = 4.dp))
        }
    }
}

/** The phone tools page's choices that don't need Compose, so they are unit-tested. */
object PhoneToolsTexts {
    enum class Shown(val tone: DotTone) { Reachable(DotTone.READY), Starting(DotTone.BUSY), Problem(DotTone.OFFLINE) }

    /** What the status line says: reachable at its URL, a problem [McpTexts.status] words, or starting. */
    fun status(status: McpStatus): Shown = when {
        status.running && status.url != null -> Shown.Reachable
        status.error != null && status.error != McpStatus.ERROR_DISABLED -> Shown.Problem
        else -> Shown.Starting
    }

    /** A port the user may pick: above the privileged ones, as SettingsStore keeps it. */
    fun port(text: String): Int? = text.toIntOrNull()?.takeIf { it in 1024..65535 }

    private val URL = Regex("\"url\"\\s*:\\s*\"([^\"]+)\"")

    /** The address in OpenClaw's setup command, for the toast that says where it points. */
    fun setupUrl(command: String): String? = URL.find(command)?.groupValues?.get(1)
}

private fun isUnrestricted(context: Context): Boolean =
    context.getSystemService(PowerManager::class.java)?.isIgnoringBatteryOptimizations(context.packageName) == true

private fun isSamsung(): Boolean = Build.MANUFACTURER.equals("samsung", ignoreCase = true)

/**
 * The app's battery page, where Samsung offers Unrestricted. Android has no public intent for it, so this
 * tries the page Settings itself opens, then the app's info page that links to it.
 */
@SuppressLint("BatteryLife")
private fun openBatterySettings(context: Context) {
    val uri = "package:${context.packageName}".toUri()
    val tries = listOf(
        Intent("android.settings.VIEW_ADVANCED_POWER_USAGE_DETAIL", uri),
        Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, uri),
    )
    for (intent in tries) {
        if (runCatching { context.startActivity(intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) }.isSuccess) return
    }
}
