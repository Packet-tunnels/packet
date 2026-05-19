package com.resolo.packet

import android.content.Context
import android.os.Build
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import java.util.TimeZone

object TunnelDiagnosticReport {
    fun build(context: Context): String {
        val snapshot = TunnelPreferences.loadSnapshot(context)
        val runtime = TunnelPreferences.loadRuntimeSnapshot(context)
        val diagnostics = TunnelPreferences.loadDiagnostics(context)
        val logs = TunnelLogStore.load(context)
        val selectedConfiguration = TunnelPreferences.loadSelectedConfigurationEntry(context)
        val activeConfiguration = TunnelPreferences.loadActiveConfiguration(context)
        val activeConfigurationName = TunnelPreferences.loadActiveConfigurationDisplayName(context)
        val configuration = if (
            runtime.tunnelActive ||
            snapshot.state == TunnelState.RUNNING ||
            snapshot.state == TunnelState.CONNECTING ||
            snapshot.state == TunnelState.DISCONNECTING
        ) {
            activeConfiguration ?: TunnelPreferences.loadConfiguration(context)
        } else {
            TunnelPreferences.loadConfiguration(context)
        }

        return buildString {
            appendLine("Packet Diagnostic Report")
            appendLine("Generated: ${timestampUtc()}")
            appendLine("App Version: ${packageVersion(context)}")
            appendLine("Device: ${Build.MANUFACTURER} ${Build.MODEL} (SDK ${Build.VERSION.SDK_INT}, Android ${Build.VERSION.RELEASE})")
            appendLine("State: ${snapshot.state.name}")
            appendLine("State Message: ${snapshot.message}")
            appendLine()
            appendLine("[Configuration]")
            appendLine("selected_configuration=${selectedConfiguration?.displayName ?: "(none)"}")
            appendLine("active_configuration=${activeConfigurationName ?: "(none)"}")
            appendLine("server_url=${configuration.normalizedServerUrl.ifBlank { "(empty)" }}")
            appendLine("transport=${configuration.transportLabel}")
            appendLine("listen_port_request=${configuration.listenPort.ifBlank { "auto" }}")
            appendLine("cdn_edge=${configuration.normalizedCdnEdge.ifBlank { "(empty)" }}")
            appendLine("host_override=${configuration.normalizedHostOverride.ifBlank { "(empty)" }}")
            appendLine("sni_override=${configuration.normalizedSniOverride.ifBlank { "(empty)" }}")
            appendLine("uses_cdn=${configuration.usesCdn}")
            appendLine("secret=${redactSecret(configuration.normalizedSecret)}")
            appendLine()
            appendLine("[Runtime]")
            appendLine("state=${runtime.state}")
            appendLine("server_host=${runtime.serverHost}")
            appendLine("cdn_edge=${runtime.cdnEdge ?: "(none)"}")
            appendLine("server_country=${runtime.serverCountryName ?: runtime.serverCountryCode ?: "(unknown)"}")
            appendLine("egress_target=${runtime.egressTarget ?: "(none)"}")
            appendLine("egress_ping_ms=${runtime.egressPingMs ?: "(none)"}")
            appendLine("listen_port=${runtime.listenPort ?: "(none)"}")
            appendLine("tunnel_active=${runtime.tunnelActive}")
            appendLine("active_streams=${runtime.activeStreams}")
            appendLine("total_streams=${runtime.totalStreams}")
            appendLine("bytes_up=${runtime.bytesUp}")
            appendLine("bytes_down=${runtime.bytesDown}")
            appendLine("connected_since=${runtime.connectedSince ?: "(none)"}")
            appendLine("last_ping_ms=${runtime.lastPingMs ?: "(none)"}")
            appendLine("last_error=${runtime.lastError ?: "(none)"}")
            appendLine()
            appendLine("[Diagnostics]")
            appendLine("endpoint_host=${diagnostics.endpointHost.ifBlank { "(empty)" }}")
            appendLine("endpoint_reachable=${diagnostics.endpointReachable?.toString() ?: "(unknown)"}")
            appendLine("endpoint_latency_ms=${diagnostics.endpointLatencyMs?.toString() ?: "(none)"}")
            appendLine("health_status=${diagnostics.healthStatus}")
            appendLine("local_proxy_ready=${diagnostics.localProxyReady}")
            appendLine("vpn_shell_ready=${diagnostics.vpnShellReady}")
            appendLine("routing_comparison=${diagnostics.routingComparison}")
            appendLine("recommendation=${diagnostics.recommendation}")
            appendLine("last_failure_detail=${diagnostics.lastFailureDetail ?: "(none)"}")
            appendLine("last_updated_ms=${diagnostics.lastUpdatedMs ?: "(none)"}")
            appendLine("vpn_disclosure_acknowledged=${TunnelPreferences.isVpnDisclosureAcknowledged(context)}")
            appendLine()
            appendLine("[Logs]")
            if (logs.isEmpty()) {
                appendLine("(no logs)")
            } else {
                logs.forEach { appendLine(it) }
            }
        }
    }

    private fun packageVersion(context: Context): String {
        return runCatching {
            val pkg = context.packageManager.getPackageInfo(context.packageName, 0)
            val versionCode = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
                pkg.longVersionCode.toString()
            } else {
                @Suppress("DEPRECATION")
                pkg.versionCode.toString()
            }
            pkg.versionName ?: versionCode
        }.getOrDefault("unknown")
    }

    private fun redactSecret(secret: String): String {
        if (secret.isBlank()) {
            return "(empty)"
        }

        if (secret.length <= 4) {
            return "${"*".repeat(secret.length)} (${secret.length} chars)"
        }

        return "${secret.take(2)}***${secret.takeLast(2)} (${secret.length} chars)"
    }

    private fun timestampUtc(): String {
        return SimpleDateFormat("yyyy-MM-dd'T'HH:mm:ss.SSS'Z'", Locale.US).apply {
            timeZone = TimeZone.getTimeZone("UTC")
        }.format(Date())
    }
}
