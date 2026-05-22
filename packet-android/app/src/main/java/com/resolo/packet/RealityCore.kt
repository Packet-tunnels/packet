package com.resolo.packet

import android.content.Context
import android.net.Uri
import org.json.JSONArray
import org.json.JSONObject
import java.io.File

object RealityCore {
    private const val XRAY_LIBRARY_NAME = "libpacket_xray.so"
    private const val STARTUP_GRACE_MS = 750L

    @Volatile private var activeProcess: Process? = null
    @Volatile private var activePort: Int? = null

    data class StartResult(
        val started: Boolean,
        val socksPort: Int?,
        val httpPort: Int?,
        val error: String?,
    )

    @Synchronized
    fun startLocalProxy(
        context: Context,
        carrierUri: String,
        requestedSocksPort: Int,
        log: (String) -> Unit,
    ): StartResult {
        stopLocalProxy(log)

        val profile = runCatching { RealityProfile.fromUri(carrierUri) }.getOrElse { error ->
            return StartResult(
                started = false,
                socksPort = null,
                httpPort = null,
                error = error.localizedMessage ?: "Invalid VLESS Reality carrier URI.",
            )
        }

        val binary = findXrayBinary(context)
            ?: return StartResult(
                started = false,
                socksPort = null,
                httpPort = null,
                error = "Xray Reality core is missing. Install it as packet-android/app/src/main/jniLibs/<abi>/$XRAY_LIBRARY_NAME and rebuild.",
            )

        val socksPort = requestedSocksPort.takeIf { it in 1024..65535 } ?: 10808
        val httpPort = companionHttpPort(socksPort)
        val configFile = writeXrayConfig(context, profile, socksPort, httpPort)

        return runCatching {
            log("[REALITY] Starting Xray VLESS Reality sidecar socks=127.0.0.1:$socksPort http=127.0.0.1:$httpPort -> ${profile.server}:${profile.serverPort}")
            val process = ProcessBuilder(binary.absolutePath, "run", "-config", configFile.absolutePath)
                .redirectErrorStream(true)
                .directory(configFile.parentFile)
                .start()
            activeProcess = process
            activePort = socksPort
            pipeProcessLogs(process, log)
            Thread {
                val exitCode = process.waitFor()
                if (activeProcess === process) {
                    activeProcess = null
                    activePort = null
                    log("[REALITY] Xray sidecar exited with code $exitCode")
                }
            }.apply {
                name = "PacketRealityExit"
                isDaemon = true
                start()
            }

            Thread.sleep(STARTUP_GRACE_MS)
            val earlyExit = exitCodeIfFinished(process)
            if (earlyExit != null) {
                activeProcess = null
                activePort = null
                StartResult(
                    started = false,
                    socksPort = null,
                    httpPort = null,
                    error = "Xray Reality core exited before the SOCKS listener was ready (code $earlyExit).",
                )
            } else {
                StartResult(started = true, socksPort = socksPort, httpPort = httpPort, error = null)
            }
        }.getOrElse { error ->
            activeProcess = null
            activePort = null
            StartResult(
                started = false,
                socksPort = null,
                httpPort = null,
                error = error.localizedMessage ?: "Failed to start Xray Reality core.",
            )
        }
    }

    @Synchronized
    fun stopLocalProxy(log: (String) -> Unit = {}) {
        val process = activeProcess ?: return
        activeProcess = null
        activePort = null
        runCatching {
            process.destroy()
            log("[REALITY] Xray sidecar stopped")
        }.onFailure { error ->
            log("[REALITY] Xray sidecar stop failed: ${error.localizedMessage ?: error.javaClass.simpleName}")
        }
    }

    private fun findXrayBinary(context: Context): File? {
        val libraryDir = context.applicationInfo.nativeLibraryDir?.let(::File)
        return libraryDir
            ?.resolve(XRAY_LIBRARY_NAME)
            ?.takeIf { it.exists() && it.canExecute() }
    }

    private fun writeXrayConfig(
        context: Context,
        profile: RealityProfile,
        socksPort: Int,
        httpPort: Int,
    ): File {
        val runtimeDir = File(context.filesDir, "reality-core").apply { mkdirs() }
        val configFile = File(runtimeDir, "xray-reality-$socksPort.json")
        configFile.writeText(buildXrayConfig(profile, socksPort, httpPort).toString(2))
        return configFile
    }

    private fun buildXrayConfig(profile: RealityProfile, socksPort: Int, httpPort: Int): JSONObject {
        val socksInbound = JSONObject()
            .put("tag", "packet-reality-in")
            .put("listen", "127.0.0.1")
            .put("port", socksPort)
            .put("protocol", "socks")
            .put(
                "settings",
                JSONObject()
                    .put("auth", "noauth")
                    .put("udp", true),
            )

        val httpInbound = JSONObject()
            .put("tag", "packet-reality-http-in")
            .put("listen", "127.0.0.1")
            .put("port", httpPort)
            .put("protocol", "http")
            .put("settings", JSONObject())

        val user = JSONObject()
            .put("id", profile.uuid)
            .put("encryption", profile.encryption)
        if (profile.flow.isNotBlank()) {
            user.put("flow", profile.flow)
        }

        val outboundSettings = JSONObject()
            .put(
                "vnext",
                JSONArray().put(
                    JSONObject()
                        .put("address", profile.server)
                        .put("port", profile.serverPort)
                        .put("users", JSONArray().put(user)),
                ),
            )

        val realitySettings = JSONObject()
            .put("serverName", profile.sni)
            .put("fingerprint", profile.fingerprint)
            .put("publicKey", profile.publicKey)
        if (profile.shortId.isNotBlank()) {
            realitySettings.put("shortId", profile.shortId)
        }
        if (profile.spiderX.isNotBlank()) {
            realitySettings.put("spiderX", profile.spiderX)
        }

        val streamSettings = JSONObject()
            .put("network", profile.network)
            .put("security", "reality")
            .put("realitySettings", realitySettings)

        val outbound = JSONObject()
            .put("tag", "packet-reality-out")
            .put("protocol", "vless")
            .put("settings", outboundSettings)
            .put("streamSettings", streamSettings)

        return JSONObject()
            .put("log", JSONObject().put("loglevel", "warning"))
            .put("inbounds", JSONArray().put(socksInbound).put(httpInbound))
            .put("outbounds", JSONArray().put(outbound))
    }

    private fun companionHttpPort(socksPort: Int): Int {
        return if (socksPort < 65535) socksPort + 1 else socksPort - 1
    }

    private fun pipeProcessLogs(process: Process, log: (String) -> Unit) {
        Thread {
            process.inputStream.bufferedReader().useLines { lines ->
                lines.forEach { line ->
                    if (line.isNotBlank()) {
                        log("[REALITY] $line")
                    }
                }
            }
        }.apply {
            name = "PacketRealityLogs"
            isDaemon = true
            start()
        }
    }

    private fun exitCodeIfFinished(process: Process): Int? {
        return try {
            process.exitValue()
        } catch (_: IllegalThreadStateException) {
            null
        }
    }

    private data class RealityProfile(
        val uuid: String,
        val server: String,
        val serverPort: Int,
        val network: String,
        val encryption: String,
        val flow: String,
        val sni: String,
        val fingerprint: String,
        val publicKey: String,
        val shortId: String,
        val spiderX: String,
    ) {
        companion object {
            fun fromUri(raw: String): RealityProfile {
                val uri = Uri.parse(raw.trim())
                require(uri.scheme.equals("vless", ignoreCase = true)) {
                    "Reality sidecar only supports vless:// carrier URIs."
                }
                require(uri.getQueryParameter("security").equals("reality", ignoreCase = true)) {
                    "Reality sidecar only handles VLESS URIs with security=reality."
                }

                val network = uri.getQueryParameter("type")
                    ?.trim()
                    ?.lowercase()
                    ?.ifBlank { null }
                    ?: "tcp"
                require(network == "tcp") {
                    "VLESS Reality sidecar currently supports type=tcp only."
                }

                val uuid = uri.userInfo?.trim().orEmpty()
                val server = uri.host?.trim().orEmpty()
                val port = uri.port.takeIf { it in 1..65535 } ?: 443
                val sni = uri.getQueryParameter("sni")?.trim().orEmpty()
                    .ifBlank { uri.getQueryParameter("serverName")?.trim().orEmpty() }
                    .ifBlank { server }
                val publicKey = uri.getQueryParameter("pbk")?.trim().orEmpty()
                    .ifBlank { uri.getQueryParameter("publicKey")?.trim().orEmpty() }

                require(uuid.isNotBlank()) { "VLESS Reality URI is missing a UUID user." }
                require(server.isNotBlank()) { "VLESS Reality URI is missing a server host." }
                require(publicKey.isNotBlank()) { "VLESS Reality URI is missing pbk/publicKey." }

                return RealityProfile(
                    uuid = uuid,
                    server = server,
                    serverPort = port,
                    network = network,
                    encryption = uri.getQueryParameter("encryption")?.trim().orEmpty().ifBlank { "none" },
                    flow = uri.getQueryParameter("flow")?.trim().orEmpty(),
                    sni = sni,
                    fingerprint = uri.getQueryParameter("fp")?.trim().orEmpty()
                        .ifBlank { uri.getQueryParameter("fingerprint")?.trim().orEmpty() }
                        .ifBlank { "chrome" },
                    publicKey = publicKey,
                    shortId = uri.getQueryParameter("sid")?.trim().orEmpty()
                        .ifBlank { uri.getQueryParameter("shortId")?.trim().orEmpty() },
                    spiderX = uri.getQueryParameter("spx")?.trim().orEmpty()
                        .ifBlank { uri.getQueryParameter("spiderX")?.trim().orEmpty() }
                        .ifBlank { "/" },
                )
            }
        }
    }
}
