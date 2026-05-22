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
    // QUIC over UDP — the escape transport that survives Iran's "RST every
    // foreign TLS over TCP" filter, because UDP/443 has to stay open for
    // WhatsApp / Meet / Telegram video calls. Auto-included in the rotation
    // pool when the active mode is AUTO.
    QUIC(6, "QUIC"),
    WEBSOCKET(1, "WebSocket"),
    HTTP(2, "HTTP"),
    STEALTH(3, "Stealth"),
    OBFS(4, "Obfs"),
    MEEK(5, "Meek");

    companion object {
        fun fromRawValue(value: Int): TunnelTransportMode {
            return values().firstOrNull { it.rawValue == value } ?: AUTO
        }
    }
}

enum class TunnelStackMode(val rawValue: Int, val title: String) {
    PACKET_NATIVE(0, "Packet Native"),
    CUSTOM_TROJAN_CARRIER(1, "DirectSock"),
    PACKET_CHAIN(2, "Packet Chain"),
    PRIVATE_RELAY(3, "Private Relay"),
    PSIPHON_CHAIN(4, "Psiphon Chain");

    companion object {
        fun fromRawValue(value: Int): TunnelStackMode {
            return values().firstOrNull { it.rawValue == value } ?: PACKET_NATIVE
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

object PacketDefaultProfiles {
    const val CHAIN_NAME = "Packet Chain"
    const val PSIPHON_CHAIN_NAME = "Psiphon Escape"
    const val CHAIN_SERVER_URL = "http://114.29.236.118:80"
    const val CHAIN_SECRET = "4ff204d5baf2f12406a45a4b2793c508f2cec2dfab865f9c8904eb5cec2024b2"
    const val CHAIN_EDGE = "114.29.236.118:80"
    const val CHAIN_OBFS_KEY = "1dbe8442ad975fb80a497d0cda4a547844cb81aefec8520e5a15055634585ee7"
    const val CHAIN_TROJAN_URI =
        "trojan://humanity@172.64.152.23:80?path=%2Fassignment&security=none" +
            "&host=www.creationlong.org&type=ws#%40InfoTech_VK"
    const val PSIPHON_LOCAL_HTTP_PORT = 18080
    const val PSIPHON_LOCAL_SOCKS_PORT = 18081

    fun chainConfiguration(): TunnelConfiguration {
        return TunnelConfiguration(
            stackMode = TunnelStackMode.PACKET_CHAIN,
            serverUrl = CHAIN_SERVER_URL,
            secret = CHAIN_SECRET,
            listenPort = "",
            cdnEdge = CHAIN_EDGE,
            hostOverride = "",
            sniOverride = "",
            // AUTO triggers the candidate-rotation supervisor on the Rust
            // side, which sweeps WS+ChromeTLS / Obfs / QUIC across every
            // port we know about — what actually escapes Iran 2026 instead
            // of being a single Meek HTTP polling shape that gets RST'd.
            transportMode = TunnelTransportMode.AUTO,
            obfsKey = CHAIN_OBFS_KEY,
            upstreamProxy = "",
            fragmentEnabled = true,
            fragmentSize = "100",
            trojanCarrierUri = CHAIN_TROJAN_URI,
            carrierProxyPort = "10808",
        )
    }

    fun psiphonChainConfiguration(): TunnelConfiguration {
        return TunnelConfiguration(
            stackMode = TunnelStackMode.PSIPHON_CHAIN,
            serverUrl = CHAIN_SERVER_URL,
            secret = CHAIN_SECRET,
            listenPort = "",
            cdnEdge = CHAIN_EDGE,
            hostOverride = "",
            sniOverride = "",
            transportMode = TunnelTransportMode.AUTO,
            obfsKey = CHAIN_OBFS_KEY,
            upstreamProxy = "",
            fragmentEnabled = true,
            fragmentSize = "100",
            trojanCarrierUri = CHAIN_TROJAN_URI,
            carrierProxyPort = "10808",
        )
    }

    // ── Packet QUIC: direct UDP/443 escape ─────────────────────────────
    // No trojan, no Cloudflare, no TCP at all. The phantom tunnel runs
    // inside a QUIC connection straight to our own server on UDP/443.
    // QUIC datagrams are indistinguishable from HTTP/3 (YouTube, Meet,
    // WhatsApp video) and — critically — UDP has no RST, so Iran's DPI
    // cannot inject the reset that kills every TCP+TLS first hop. This is
    // the escape the trojan/CF chain structurally cannot achieve.
    const val QUIC_NAME = "Packet QUIC"
    const val QUIC_SERVER_URL = "http://114.29.236.118:80"
    // cdnEdge is the actual UDP dial target — the server's QUIC listener.
    const val QUIC_EDGE = "114.29.236.118:443"

    fun quicConfiguration(): TunnelConfiguration {
        return TunnelConfiguration(
            stackMode = TunnelStackMode.PACKET_NATIVE,
            serverUrl = QUIC_SERVER_URL,
            secret = CHAIN_SECRET,
            listenPort = "",
            cdnEdge = QUIC_EDGE,
            hostOverride = "",
            sniOverride = "",
            transportMode = TunnelTransportMode.QUIC,
            obfsKey = "",
            upstreamProxy = "",
            // QUIC has no TLS-ClientHello fragmentation concept; the QUIC
            // Initial packet is already its own obfuscation surface.
            fragmentEnabled = false,
            fragmentSize = "0",
            trojanCarrierUri = "",
            carrierProxyPort = "",
        )
    }
}

data class TunnelConfiguration(
    val stackMode: TunnelStackMode = TunnelStackMode.PACKET_NATIVE,
    val serverUrl: String = "",
    val secret: String = "",
    val listenPort: String = "",
    val cdnEdge: String = "",
    val hostOverride: String = "",
    val sniOverride: String = "",
    val transportMode: TunnelTransportMode = TunnelTransportMode.AUTO,
    val obfsKey: String = "",
    val upstreamProxy: String = "",
    val fragmentEnabled: Boolean = true,
    val fragmentSize: String = "40",
    val trojanCarrierUri: String = "",
    val carrierProxyPort: String = "10808",
) {
    val isEmpty: Boolean
        get() = stackMode == TunnelStackMode.PACKET_NATIVE &&
            normalizedServerUrl.isEmpty() &&
            normalizedSecret.isEmpty() &&
            listenPort.trim().isEmpty() &&
            normalizedCdnEdge.isEmpty() &&
            normalizedHostOverride.isEmpty() &&
            normalizedSniOverride.isEmpty() &&
            normalizedObfsKey.isEmpty() &&
            normalizedUpstreamProxy.isEmpty() &&
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

    val normalizedObfsKey: String
        get() = obfsKey.trim()

    val normalizedUpstreamProxy: String
        get() = upstreamProxy.trim()

    val normalizedTrojanCarrierUri: String
        get() = trojanCarrierUri.trim()

    private val parsedServerUri: Uri?
        get() = runCatching { Uri.parse(normalizedServerUrl) }.getOrNull()

    private val parsedServerHost: String
        get() = parsedServerUri?.host.orEmpty()

    private val parsedServerPort: Int?
        get() = parsedServerUri?.port?.takeIf { it > 0 }

    val usesCustomCarrier: Boolean
        get() = stackMode == TunnelStackMode.CUSTOM_TROJAN_CARRIER

    val usesPacketChain: Boolean
        get() = stackMode == TunnelStackMode.PACKET_CHAIN

    val usesPsiphonChain: Boolean
        get() = stackMode == TunnelStackMode.PSIPHON_CHAIN

    val usesPrivateRelay: Boolean
        get() = stackMode == TunnelStackMode.PRIVATE_RELAY

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
        get() = usesCustomCarrier ||
            usesPacketChain ||
            usesPsiphonChain ||
            usesPrivateRelay ||
            usesCdn ||
            transportMode != TunnelTransportMode.AUTO ||
            normalizedObfsKey.isNotEmpty() ||
            normalizedUpstreamProxy.isNotEmpty() ||
            fragmentEnabled

    val ingressLabel: String
        get() = when {
            usesPrivateRelay -> "Private Starlink relay"
            usesPsiphonChain -> "Trojan + Psiphon + Packet"
            usesPacketChain -> "Trojan + Packet"
            usesCustomCarrier -> "DirectSock"
            transportMode == TunnelTransportMode.OBFS -> "Obfs raw TCP"
            transportMode == TunnelTransportMode.MEEK -> "Meek HTTP"
            transportMode == TunnelTransportMode.STEALTH -> "Stealth TLS"
            normalizedSniOverride.isNotEmpty() -> "SNI fronting"
            normalizedCdnEdge.isNotEmpty() || normalizedHostOverride.isNotEmpty() -> "CDN relay"
            else -> "Standard endpoint"
        }

    val carrierProxyPortValue: Int
        get() = carrierProxyPort.trim().toIntOrNull()?.takeIf { it in 1024..65535 } ?: 10808

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
            if (usesCustomCarrier) {
                return carrierEndpointHost
            }

            if (usesPacketChain) {
                return carrierEndpointHost
            }

            if (usesPsiphonChain) {
                return carrierEndpointHost
            }

            if (usesPrivateRelay) {
                return parsedServerHost
            }

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
            if (usesCustomCarrier) {
                return carrierEndpointHost
            }

            if (usesPacketChain) {
                return carrierEndpointHost
            }

            if (usesPsiphonChain) {
                return carrierEndpointHost
            }

            if (usesPrivateRelay) {
                return parsedServerHost
            }

            if (cdnEdgeValidationError != null) {
                return serverHost
            }
            val edgeHost = normalizedCdnEdge.substringBefore(":").trim()
            return if (edgeHost.isNotEmpty()) edgeHost else serverHost
        }

    val endpointPort: Int
        get() {
            if (usesCustomCarrier) {
                return carrierEndpointPort
            }

            if (usesPacketChain) {
                return carrierEndpointPort
            }

            if (usesPsiphonChain) {
                return carrierEndpointPort
            }

            if (usesPrivateRelay) {
                return parsedServerPort ?: 80
            }

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

    private val carrierEndpointHost: String
        get() {
            return runCatching { Uri.parse(normalizedTrojanCarrierUri).host }
                .getOrNull()
                ?.takeIf { it.isNotBlank() }
                ?: "Unavailable"
        }

    private val carrierEndpointPort: Int
        get() {
            return runCatching { Uri.parse(normalizedTrojanCarrierUri).port }
                .getOrNull()
                ?.takeIf { it > 0 }
                ?: 443
        }

    val transportLabel: String
        get() = transportMode.title

    val suggestedName: String
        get() {
            if (usesCustomCarrier) {
                return "DirectSock"
            }

            if (usesPacketChain) {
                return PacketDefaultProfiles.CHAIN_NAME
            }

            if (usesPsiphonChain) {
                return PacketDefaultProfiles.PSIPHON_CHAIN_NAME
            }

            if (usesPrivateRelay) {
                return "Private Relay"
            }

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
            if (usesCustomCarrier) {
                if (normalizedTrojanCarrierUri.isEmpty()) {
                    return "Trojan URI is required for DirectSock mode."
                }
                if (!normalizedTrojanCarrierUri.startsWith("trojan://", ignoreCase = true)) {
                    return "DirectSock URI must start with trojan://."
                }
                if (carrierProxyPort.trim().toIntOrNull()?.takeIf { it in 1024..65535 } == null) {
                    return "DirectSock local port must be 1024-65535."
                }
                return null
            }

            if (usesPacketChain) {
                if (normalizedTrojanCarrierUri.isEmpty()) {
                    return "Trojan URI is required for Packet Chain mode."
                }
                if (!normalizedTrojanCarrierUri.startsWith("trojan://", ignoreCase = true)) {
                    return "Packet Chain Trojan URI must start with trojan://."
                }
                if (carrierProxyPort.trim().toIntOrNull()?.takeIf { it in 1024..65535 } == null) {
                    return "Packet Chain carrier port must be 1024-65535."
                }
                if (normalizedServerUrl.isEmpty()) {
                    return "Packet Chain server URL is required."
                }
                if (normalizedSecret.isEmpty()) {
                    return "Packet Chain shared secret is required."
                }
                cdnEdgeValidationError?.let { return it }
                return null
            }

            if (usesPsiphonChain) {
                if (normalizedTrojanCarrierUri.isEmpty()) {
                    return "Trojan URI is required for Psiphon Chain mode."
                }
                if (!normalizedTrojanCarrierUri.startsWith("trojan://", ignoreCase = true)) {
                    return "Psiphon Chain Trojan URI must start with trojan://."
                }
                if (carrierProxyPort.trim().toIntOrNull()?.takeIf { it in 1024..65535 } == null) {
                    return "Psiphon Chain carrier port must be 1024-65535."
                }
                if (normalizedServerUrl.isEmpty()) {
                    return "Psiphon Chain Packet server URL is required."
                }
                if (normalizedSecret.isEmpty()) {
                    return "Psiphon Chain Packet shared secret is required."
                }
                cdnEdgeValidationError?.let { return it }
                return null
            }

            if (usesPrivateRelay) {
                if (normalizedServerUrl.isEmpty()) {
                    return "Private Relay server URL is required."
                }
                if (normalizedSecret.isEmpty()) {
                    return "Private Relay shared secret is required."
                }
                val scheme = runCatching { Uri.parse(normalizedServerUrl).scheme?.lowercase() }.getOrNull()
                if (scheme !in setOf("http", "https")) {
                    return "Private Relay server URL must start with http:// or https://."
                }
                if (parsedServerHost.isBlank()) {
                    return "Private Relay server URL is missing a host."
                }
                return null
            }

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

            if (transportMode == TunnelTransportMode.OBFS && normalizedCdnEdge.isEmpty()) {
                return "Obfs transport requires CDN Edge set to the direct server IP:port, for example 103.241.67.247:36571."
            }

            upstreamProxyValidationError?.let { return it }

            return null
        }

    val upstreamProxyValidationError: String?
        get() {
            val proxy = normalizedUpstreamProxy
            if (proxy.isEmpty()) return null

            val uri = runCatching { Uri.parse(proxy) }.getOrNull()
                ?: return "First-hop proxy must be socks5://host:port or http://host:port."
            val scheme = uri.scheme?.lowercase()
            if (scheme !in setOf("socks", "socks5", "http", "https")) {
                return "First-hop proxy must be socks5://host:port or http://host:port."
            }
            if (uri.host.isNullOrBlank()) {
                return "First-hop proxy is missing a host."
            }
            val port = uri.port
            if (port !in 1..65535) {
                return "First-hop proxy port must be between 1 and 65535."
            }

            return null
        }

    fun toJsonObject(): JSONObject {
        return JSONObject()
            .put("stack_mode", stackMode.rawValue)
            .put("server_url", serverUrl)
            .put("secret", secret)
            .put("listen_port", listenPort)
            .put("cdn_edge", cdnEdge)
            .put("host_override", hostOverride)
            .put("sni_override", sniOverride)
            .put("transport_mode", transportMode.rawValue)
            .put("obfs_key", obfsKey)
            .put("upstream_proxy", upstreamProxy)
            .put("fragment_enabled", fragmentEnabled)
            .put("fragment_size", fragmentSize)
            .put("trojan_carrier_uri", trojanCarrierUri)
            .put("carrier_proxy_port", carrierProxyPort)
    }

    companion object {
        fun fromJsonObject(json: JSONObject): TunnelConfiguration {
            return TunnelConfiguration(
                stackMode = TunnelStackMode.fromRawValue(
                    json.optInt("stack_mode", TunnelStackMode.PACKET_NATIVE.rawValue)
                ),
                serverUrl = json.optString("server_url", ""),
                secret = json.optString("secret", ""),
                listenPort = json.optString("listen_port", ""),
                cdnEdge = json.optString("cdn_edge", ""),
                hostOverride = json.optString("host_override", ""),
                sniOverride = json.optString("sni_override", ""),
                transportMode = TunnelTransportMode.fromRawValue(
                    json.optInt("transport_mode", TunnelTransportMode.AUTO.rawValue)
                ),
                obfsKey = json.optString("obfs_key", ""),
                upstreamProxy = json.optString("upstream_proxy", ""),
                fragmentEnabled = json.optBoolean("fragment_enabled", true),
                fragmentSize = json.optString("fragment_size", "40"),
                trojanCarrierUri = json.optString("trojan_carrier_uri", ""),
                carrierProxyPort = json.optString("carrier_proxy_port", "10808"),
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
        get() = if (configuration.usesCustomCarrier) {
            configuration.normalizedTrojanCarrierUri.ifBlank { "DirectSock not configured" }
        } else if (configuration.usesPacketChain) {
            "${configuration.normalizedTrojanCarrierUri.ifBlank { "Trojan missing" }} -> ${configuration.normalizedCdnEdge.ifBlank { "Packet edge missing" }}"
        } else if (configuration.usesPsiphonChain) {
            "${configuration.normalizedTrojanCarrierUri.ifBlank { "Trojan missing" }} -> Psiphon -> ${configuration.normalizedCdnEdge.ifBlank { "Packet edge missing" }}"
        } else if (configuration.usesPrivateRelay) {
            "${configuration.normalizedServerUrl.ifBlank { "Private VPS missing" }} -> Starlink relay"
        } else {
            configuration.normalizedServerUrl.ifBlank { "Not configured" }
        }

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
    val serverCountryCode: String? = null,
    val serverCountryName: String? = null,
    val egressPingMs: Int? = null,
    val egressTarget: String? = null,
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
            .put("server_country_code", serverCountryCode)
            .put("server_country_name", serverCountryName)
            .put("egress_ping_ms", egressPingMs)
            .put("egress_target", egressTarget)
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
                    serverCountryCode = json.optNullableString("server_country_code"),
                    serverCountryName = json.optNullableString("server_country_name"),
                    egressPingMs = json.optInt("egress_ping_ms").takeIf { it > 0 },
                    egressTarget = json.optNullableString("egress_target"),
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
