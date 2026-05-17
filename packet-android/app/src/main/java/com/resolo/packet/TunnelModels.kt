package com.resolo.packet

import android.net.Uri
import org.json.JSONArray
import org.json.JSONObject
import java.util.UUID

object TunnelActions {
    const val ACTION_CONNECT = "com.resolo.packet.action.CONNECT"
    const val ACTION_DISCONNECT = "com.resolo.packet.action.DISCONNECT"
    const val ACTION_LOG_UPDATED = "com.resolo.packet.action.LOG_UPDATED"
    const val ACTION_STATE_UPDATED = "com.resolo.packet.action.STATE_UPDATED"
    const val ACTION_DASHBOARD_UPDATED = "com.resolo.packet.action.DASHBOARD_UPDATED"
}

enum class TunnelTransportMode(val rawValue: Int, val title: String) {
    AUTO(0, "Auto"),
    WEBSOCKET(1, "WebSocket"),
    HTTP(2, "HTTP"),
    STEALTH(3, "Stealth");

    companion object {
        fun fromRawValue(value: Int): TunnelTransportMode {
            return values().firstOrNull { it.rawValue == value } ?: AUTO
        }
    }
}

enum class TunnelState(val title: String) {
    IDLE("Idle"),
    REQUESTING_PERMISSION("Requesting Permission"),
    CONNECTING("Connecting"),
    RUNNING("Running"),
    DISCONNECTING("Disconnecting"),
    FAILED("Failed");

    val isActive: Boolean
        get() = this == REQUESTING_PERMISSION || this == CONNECTING || this == RUNNING || this == DISCONNECTING
}

data class TunnelConfiguration(
    val serverUrl: String = "",
    val secret: String = "",
    val listenPort: String = "",
    val cdnEdge: String = "",
    val hostOverride: String = "",
    val sniOverride: String = "",
    val transportMode: TunnelTransportMode = TunnelTransportMode.AUTO,
    val fragmentEnabled: Boolean = false,
    val fragmentSize: String = "40",
) {
    val isEmpty: Boolean
        get() = normalizedServerUrl.isEmpty() &&
            normalizedSecret.isEmpty() &&
            listenPort.trim().isEmpty() &&
            normalizedCdnEdge.isEmpty() &&
            normalizedHostOverride.isEmpty() &&
            normalizedSniOverride.isEmpty() &&
            transportMode == TunnelTransportMode.AUTO

    val normalizedServerUrl: String
        get() = serverUrl.trim()

    val normalizedSecret: String
        get() = secret.trim()

    val normalizedCdnEdge: String
        get() = cdnEdge.trim()

    val normalizedHostOverride: String
        get() = hostOverride.trim()

    val normalizedSniOverride: String
        get() = sniOverride.trim()

    val cdnEdgeValidationError: String?
        get() {
            val edge = normalizedCdnEdge
            if (edge.isEmpty()) {
                return null
            }

            if (edge.all(Char::isDigit)) {
                return "CDN edge must be a host or IP, optionally with :port. If you only need a custom origin port, add it to Server URL instead."
            }

            if (edge.startsWith(":") || edge.endsWith(":")) {
                return "CDN edge must look like 185.143.234.235:80 or edge.example.ir."
            }

            val lastColonIndex = edge.lastIndexOf(':')
            if (lastColonIndex > 0 && edge.indexOf(':') == lastColonIndex) {
                val port = edge.substring(lastColonIndex + 1)
                val portValue = port.toIntOrNull()
                if (port.any { !it.isDigit() } || portValue == null || portValue !in 1..65535) {
                    return "CDN edge port must be between 1 and 65535."
                }
            }

            return null
        }

    val usesCdn: Boolean
        get() = normalizedCdnEdge.isNotEmpty() ||
            normalizedHostOverride.isNotEmpty() ||
            normalizedSniOverride.isNotEmpty()

    val usesAdvancedStart: Boolean
        get() = usesCdn || transportMode != TunnelTransportMode.AUTO || fragmentEnabled

    val ingressLabel: String
        get() = when {
            transportMode == TunnelTransportMode.STEALTH -> "Stealth TLS"
            normalizedSniOverride.isNotEmpty() -> "SNI fronting"
            normalizedCdnEdge.isNotEmpty() || normalizedHostOverride.isNotEmpty() -> "CDN relay"
            else -> "Standard endpoint"
        }

    val listenPortValue: Int?
        get() {
            val trimmed = listenPort.trim()
            if (trimmed.isEmpty() || trimmed.equals("auto", ignoreCase = true)) {
                return null
            }

            return trimmed.toIntOrNull()?.takeIf { it in 1024..65535 }
        }

    val fragmentSizeValue: Int
        get() {
            val default = 40
            if (!fragmentEnabled) return default
            return fragmentSize.trim().toIntOrNull()?.takeIf { it in 1..1000 } ?: default
        }

    val serverHost: String
        get() {
            val parsedHost = runCatching { Uri.parse(normalizedServerUrl).host }.getOrNull()
            if (!parsedHost.isNullOrBlank()) {
                return parsedHost
            }

            return normalizedServerUrl
                .removePrefix("http://")
                .removePrefix("https://")
                .substringBefore("/")
                .substringBefore(":")
                .ifBlank { "Unavailable" }
        }

    val endpointHost: String
        get() {
            if (cdnEdgeValidationError != null) {
                return serverHost
            }
            val edgeHost = normalizedCdnEdge.substringBefore(":").trim()
            return if (edgeHost.isNotEmpty()) edgeHost else serverHost
        }

    val endpointPort: Int
        get() {
            if (cdnEdgeValidationError != null) {
                val parsedPort = runCatching { Uri.parse(normalizedServerUrl).port }.getOrNull()
                if (parsedPort != null && parsedPort > 0) {
                    return parsedPort
                }

                return if (normalizedServerUrl.startsWith("https")) 443 else 80
            }

            val edgePort = normalizedCdnEdge.substringAfter(":", "").trim().toIntOrNull()
            if (edgePort != null) {
                return edgePort
            }

            val parsedPort = runCatching { Uri.parse(normalizedServerUrl).port }.getOrNull()
            if (parsedPort != null && parsedPort > 0) {
                return parsedPort
            }

            return if (normalizedServerUrl.startsWith("https")) 443 else 80
        }

    val transportLabel: String
        get() = transportMode.title

    val suggestedName: String
        get() {
            if (normalizedHostOverride.isNotEmpty()) {
                return normalizedHostOverride
            }

            val edgeHost = normalizedCdnEdge.substringBefore(":").trim()
            if (edgeHost.isNotEmpty()) {
                return edgeHost
            }

            if (normalizedServerUrl.isNotEmpty()) {
                return serverHost
            }

            return "New Server"
        }

    val validationError: String?
        get() {
            if (normalizedServerUrl.isEmpty()) {
                return "Server URL is required."
            }

            if (normalizedSecret.isEmpty()) {
                return "Shared secret is required."
            }

            val trimmedPort = listenPort.trim()
            if (trimmedPort.isNotEmpty() &&
                !trimmedPort.equals("auto", ignoreCase = true) &&
                listenPortValue == null
            ) {
                return "Listen port must be 1024-65535, or leave it blank for auto."
            }

            cdnEdgeValidationError?.let { return it }

            if (normalizedSniOverride.isNotEmpty() &&
                !normalizedServerUrl.startsWith("https://", ignoreCase = true)
            ) {
                return "SNI override requires an https:// server URL."
            }

            if (transportMode == TunnelTransportMode.STEALTH &&
                !normalizedServerUrl.startsWith("https://", ignoreCase = true)
            ) {
                return "Stealth transport requires an https:// server URL."
            }

            return null
        }

    fun toJsonObject(): JSONObject {
        return JSONObject()
            .put("server_url", serverUrl)
            .put("secret", secret)
            .put("listen_port", listenPort)
            .put("cdn_edge", cdnEdge)
            .put("host_override", hostOverride)
            .put("sni_override", sniOverride)
            .put("transport_mode", transportMode.rawValue)
    }

    companion object {
        fun fromJsonObject(json: JSONObject): TunnelConfiguration {
            return TunnelConfiguration(
                serverUrl = json.optString("server_url", ""),
                secret = json.optString("secret", ""),
                listenPort = json.optString("listen_port", ""),
                cdnEdge = json.optString("cdn_edge", ""),
                hostOverride = json.optString("host_override", ""),
                sniOverride = json.optString("sni_override", ""),
                transportMode = TunnelTransportMode.fromRawValue(
                    json.optInt("transport_mode", TunnelTransportMode.AUTO.rawValue)
                ),
            )
        }
    }
}

data class SavedTunnelConfiguration(
    val id: String = UUID.randomUUID().toString(),
    val name: String = "",
    val configuration: TunnelConfiguration = TunnelConfiguration(),
) {
    val trimmedName: String
        get() = name.trim()

    val displayName: String
        get() = trimmedName.ifEmpty { configuration.suggestedName }

    val subtitle: String
        get() = configuration.normalizedServerUrl.ifBlank { "Not configured" }

    fun toJsonObject(): JSONObject {
        return JSONObject()
            .put("id", id)
            .put("name", name)
            .put("configuration", configuration.toJsonObject())
    }

    companion object {
        fun fromJsonObject(json: JSONObject): SavedTunnelConfiguration {
            return SavedTunnelConfiguration(
                id = json.optString("id", UUID.randomUUID().toString()),
                name = json.optString("name", ""),
                configuration = TunnelConfiguration.fromJsonObject(
                    json.optJSONObject("configuration") ?: JSONObject()
                ),
            )
        }
    }
}

private fun JSONArray.toSavedConfigurationList(): List<SavedTunnelConfiguration> {
    return buildList {
        for (index in 0 until length()) {
            val item = optJSONObject(index) ?: continue
            add(SavedTunnelConfiguration.fromJsonObject(item))
        }
    }
}

private fun JSONObject.optNullableString(key: String): String? {
    if (!has(key) || isNull(key)) {
        return null
    }

    return optString(key)
        .takeIf { it.isNotBlank() && !it.equals("null", ignoreCase = true) }
}

data class TunnelSnapshot(
    val state: TunnelState = TunnelState.IDLE,
    val message: String = "Ready",
    val updatedAtMs: Long = 0,
)

data class TunnelRuntimeSnapshot(
    val state: String = "idle",
    val transport: String = "Unknown",
    val serverHost: String = "",
    val cdnEdge: String? = null,
    val listenPort: Int? = null,
    val bytesUp: Long = 0,
    val bytesDown: Long = 0,
    val activeStreams: Int = 0,
    val totalStreams: Long = 0,
    val connectedSince: Long? = null,
    val lastPingMs: Int? = null,
    val lastError: String? = null,
    val tunnelActive: Boolean = false,
) {
    val endpointHost: String
        get() = if (!cdnEdge.isNullOrBlank()) cdnEdge.substringBefore(":") else serverHost

    fun toJsonString(): String {
        return JSONObject()
            .put("state", state)
            .put("transport", transport)
            .put("server_host", serverHost)
            .put("cdn_edge", cdnEdge)
            .put("listen_port", listenPort)
            .put("bytes_up", bytesUp)
            .put("bytes_down", bytesDown)
            .put("active_streams", activeStreams)
            .put("total_streams", totalStreams)
            .put("connected_since", connectedSince)
            .put("last_ping_ms", lastPingMs)
            .put("last_error", lastError)
            .put("tunnel_active", tunnelActive)
            .toString()
    }

    companion object {
        val empty = TunnelRuntimeSnapshot()

        fun fromJsonString(raw: String?): TunnelRuntimeSnapshot {
            if (raw.isNullOrBlank()) {
                return empty
            }

            return runCatching {
                val json = JSONObject(raw)
                TunnelRuntimeSnapshot(
                    state = json.optString("state", "idle"),
                    transport = json.optString("transport", "Unknown"),
                    serverHost = json.optString("server_host", ""),
                    cdnEdge = json.optNullableString("cdn_edge"),
                    listenPort = json.optInt("listen_port").takeIf { it > 0 },
                    bytesUp = json.optLong("bytes_up", 0),
                    bytesDown = json.optLong("bytes_down", 0),
                    activeStreams = json.optInt("active_streams", 0),
                    totalStreams = json.optLong("total_streams", 0),
                    connectedSince = json.optLong("connected_since").takeIf { it > 0 },
                    lastPingMs = json.optInt("last_ping_ms").takeIf { it > 0 },
                    lastError = json.optNullableString("last_error"),
                    tunnelActive = json.optBoolean("tunnel_active", false),
                )
            }.getOrDefault(empty)
        }
    }
}

data class TunnelDiagnosticsSnapshot(
    val endpointHost: String = "",
    val endpointReachable: Boolean? = null,
    val endpointLatencyMs: Int? = null,
    val healthStatus: String = "Not checked",
    val localProxyReady: Boolean = false,
    val vpnShellReady: Boolean = false,
    val routingComparison: String = "",
    val recommendation: String = "",
    val lastFailureDetail: String? = null,
    val lastUpdatedMs: Long? = null,
) {
    fun toJsonString(): String {
        return JSONObject()
            .put("endpoint_host", endpointHost)
            .put("endpoint_reachable", endpointReachable)
            .put("endpoint_latency_ms", endpointLatencyMs)
            .put("health_status", healthStatus)
            .put("local_proxy_ready", localProxyReady)
            .put("vpn_shell_ready", vpnShellReady)
            .put("routing_comparison", routingComparison)
            .put("recommendation", recommendation)
            .put("last_failure_detail", lastFailureDetail)
            .put("last_updated_ms", lastUpdatedMs)
            .toString()
    }

    companion object {
        val empty = TunnelDiagnosticsSnapshot()

        fun fromJsonString(raw: String?): TunnelDiagnosticsSnapshot {
            if (raw.isNullOrBlank()) {
                return empty
            }

            return runCatching {
                val json = JSONObject(raw)
                TunnelDiagnosticsSnapshot(
                    endpointHost = json.optString("endpoint_host", ""),
                    endpointReachable = when {
                        !json.has("endpoint_reachable") -> null
                        json.isNull("endpoint_reachable") -> null
                        else -> json.optBoolean("endpoint_reachable")
                    },
                    endpointLatencyMs = json.optInt("endpoint_latency_ms").takeIf { it > 0 },
                    healthStatus = json.optString("health_status", "Not checked"),
                    localProxyReady = json.optBoolean("local_proxy_ready", false),
                    vpnShellReady = json.optBoolean("vpn_shell_ready", false),
                    routingComparison = json.optString("routing_comparison", ""),
                    recommendation = json.optString("recommendation", ""),
                    lastFailureDetail = json.optNullableString("last_failure_detail"),
                    lastUpdatedMs = json.optLong("last_updated_ms").takeIf { it > 0 },
                )
            }.getOrDefault(empty)
        }
    }
}
