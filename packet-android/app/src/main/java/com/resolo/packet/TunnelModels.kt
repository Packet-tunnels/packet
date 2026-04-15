package com.resolo.packet

import android.net.Uri
import org.json.JSONObject

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
    HTTP(2, "HTTP");

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
) {
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

    val ingressLabel: String
        get() = when {
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
}

data class TunnelSnapshot(
    val state: TunnelState = TunnelState.IDLE,
    val message: String = "Ready",
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
                    cdnEdge = json.optString("cdn_edge").takeIf { it.isNotBlank() },
                    listenPort = json.optInt("listen_port").takeIf { it > 0 },
                    bytesUp = json.optLong("bytes_up", 0),
                    bytesDown = json.optLong("bytes_down", 0),
                    activeStreams = json.optInt("active_streams", 0),
                    totalStreams = json.optLong("total_streams", 0),
                    connectedSince = json.optLong("connected_since").takeIf { it > 0 },
                    lastPingMs = json.optInt("last_ping_ms").takeIf { it > 0 },
                    lastError = json.optString("last_error").takeIf { it.isNotBlank() },
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
                    lastFailureDetail = json.optString("last_failure_detail").takeIf { it.isNotBlank() },
                    lastUpdatedMs = json.optLong("last_updated_ms").takeIf { it > 0 },
                )
            }.getOrDefault(empty)
        }
    }
}
