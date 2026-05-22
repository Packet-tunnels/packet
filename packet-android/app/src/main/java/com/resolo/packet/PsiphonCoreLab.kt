package com.resolo.packet

import android.content.Context
import org.json.JSONObject
import java.lang.reflect.InvocationHandler
import java.lang.reflect.Method
import java.lang.reflect.Proxy
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicInteger

/**
 * Optional Psiphon core evaluation hook.
 *
 * The app builds without Psiphon by default. If
 * scripts/fetch-psiphon-android-aar.sh copies psiphon-tunnel-core.aar into
 * app/libs, this reflection check flips to available without forcing the
 * normal Packet build to depend on Psiphon classes.
 */
object PsiphonCoreLab {
    private const val TUNNEL_CLASS = "ca.psiphon.PsiphonTunnel"
    private const val HOST_SERVICE_CLASS = "ca.psiphon.PsiphonTunnel\$HostService"
    private const val PSI_CLASS = "psi.Psi"
    private const val CLIENT_CONFIG_ASSET = "psiphon/client.config"
    private const val START_PROXY_TIMEOUT_MS = 30_000L

    @Volatile private var activeTunnel: Any? = null
    @Volatile private var activeState: RuntimeState? = null

    data class Status(
        val androidCoreAvailable: Boolean,
        val embeddedClientConfig: Boolean,
        val buildInfo: String?,
    ) {
        val reportLine: String
            get() = buildString {
                append("android_core_available=")
                append(androidCoreAvailable)
                append(" embedded_client_config=")
                append(embeddedClientConfig)
                if (!buildInfo.isNullOrBlank()) {
                    append(" build_info=")
                    append(buildInfo.replace('\n', ' '))
                }
            }
    }

    data class StartResult(
        val started: Boolean,
        val httpPort: Int?,
        val socksPort: Int?,
        val connected: Boolean,
        val error: String?,
    )

    private class RuntimeState {
        val httpPort = AtomicInteger(0)
        val socksPort = AtomicInteger(0)
        val connected = AtomicBoolean(false)
        val proxyReady = CountDownLatch(1)
    }

    fun status(context: Context): Status {
        val coreAvailable = runCatching {
            Class.forName(TUNNEL_CLASS)
            true
        }.getOrDefault(false)

        val buildInfo = if (coreAvailable) {
            runCatching {
                Class.forName(PSI_CLASS)
                    .getMethod("getBuildInfo")
                    .invoke(null)
                    ?.toString()
            }.getOrNull()
        } else {
            null
        }

        val configAvailable = runCatching {
            context.assets.open(CLIENT_CONFIG_ASSET).use { true }
        }.getOrDefault(false)

        return Status(
            androidCoreAvailable = coreAvailable,
            embeddedClientConfig = configAvailable,
            buildInfo = buildInfo,
        )
    }

    @Synchronized
    fun startLocalProxy(
        context: Context,
        upstreamProxyUrl: String,
        requestedHttpPort: Int,
        requestedSocksPort: Int,
        log: (String) -> Unit,
    ): StartResult {
        stopLocalProxy(log)

        if (!status(context).androidCoreAvailable) {
            return StartResult(
                started = false,
                httpPort = null,
                socksPort = null,
                connected = false,
                error = "Psiphon Android core AAR is missing. Run packet-android/scripts/fetch-psiphon-android-aar.sh and rebuild.",
            )
        }

        val configJson = runCatching {
            buildClientConfig(
                context = context,
                upstreamProxyUrl = upstreamProxyUrl,
                requestedHttpPort = requestedHttpPort,
                requestedSocksPort = requestedSocksPort,
            )
        }.getOrElse { error ->
            return StartResult(
                started = false,
                httpPort = null,
                socksPort = null,
                connected = false,
                error = error.localizedMessage ?: "Failed to load Psiphon client config.",
            )
        }

        val state = RuntimeState()
        val appContext = context.applicationContext
        val hostServiceClass = Class.forName(HOST_SERVICE_CLASS)
        val hostService = Proxy.newProxyInstance(
            hostServiceClass.classLoader,
            arrayOf(hostServiceClass),
            PsiphonHostInvocationHandler(
                context = appContext,
                configJson = configJson,
                state = state,
                log = log,
            ),
        )

        val tunnelClass = Class.forName(TUNNEL_CLASS)
        val tunnel = tunnelClass
            .getMethod("newPsiphonTunnel", hostServiceClass)
            .invoke(null, hostService)

        runCatching {
            tunnelClass.getMethod("setVpnMode", Boolean::class.javaPrimitiveType).invoke(tunnel, false)
        }

        activeTunnel = tunnel
        activeState = state

        return runCatching {
            tunnelClass.getMethod("startTunneling", String::class.java).invoke(tunnel, "")
            val proxyReady = state.proxyReady.await(START_PROXY_TIMEOUT_MS, TimeUnit.MILLISECONDS)
            val httpPort = state.httpPort.get().takeIf { it > 0 }
            if (!proxyReady || httpPort == null) {
                StartResult(
                    started = false,
                    httpPort = httpPort,
                    socksPort = state.socksPort.get().takeIf { it > 0 },
                    connected = state.connected.get(),
                    error = "Psiphon core started but no local HTTP proxy was reported within ${START_PROXY_TIMEOUT_MS / 1000}s.",
                )
            } else {
                StartResult(
                    started = true,
                    httpPort = httpPort,
                    socksPort = state.socksPort.get().takeIf { it > 0 },
                    connected = state.connected.get(),
                    error = null,
                )
            }
        }.getOrElse { error ->
            activeTunnel = null
            activeState = null
            StartResult(
                started = false,
                httpPort = null,
                socksPort = null,
                connected = false,
                error = error.cause?.localizedMessage
                    ?: error.localizedMessage
                    ?: "Failed to start Psiphon core.",
            )
        }
    }

    @Synchronized
    fun stopLocalProxy(log: (String) -> Unit = {}) {
        val tunnel = activeTunnel ?: return
        runCatching {
            tunnel.javaClass.getMethod("stop").invoke(tunnel)
            log("[PSIPHON] Psiphon core stopped")
        }.onFailure { error ->
            log("[PSIPHON] Psiphon core stop failed: ${error.localizedMessage ?: error.javaClass.simpleName}")
        }
        activeTunnel = null
        activeState = null
    }

    private fun buildClientConfig(
        context: Context,
        upstreamProxyUrl: String,
        requestedHttpPort: Int,
        requestedSocksPort: Int,
    ): String {
        val raw = context.assets.open(CLIENT_CONFIG_ASSET).bufferedReader().use { it.readText() }
        val json = JSONObject(raw)
        json.put("UpstreamProxyURL", upstreamProxyUrl)
        json.put("LocalHttpProxyPort", requestedHttpPort)
        json.put("LocalSocksProxyPort", requestedSocksPort)
        json.put("EmitDiagnosticNotices", true)
        json.put("EmitBytesTransferred", true)
        json.put("EstablishTunnelTimeoutSeconds", 0)
        return json.toString()
    }

    private class PsiphonHostInvocationHandler(
        private val context: Context,
        private val configJson: String,
        private val state: RuntimeState,
        private val log: (String) -> Unit,
    ) : InvocationHandler {
        override fun invoke(proxy: Any, method: Method, args: Array<out Any?>?): Any? {
            return when (method.name) {
                "getAppName" -> "Packet"
                "getContext" -> context
                "getPsiphonConfig" -> configJson
                "getVpnService", "newVpnServiceBuilder" -> null
                "getPrimaryDnsServer", "getSecondaryDnsServer" -> ""
                "loadLibrary" -> {
                    val library = args?.firstOrNull()?.toString()
                    if (!library.isNullOrBlank()) {
                        System.loadLibrary(library)
                    }
                    null
                }
                "onDiagnosticMessage" -> {
                    log("[PSIPHON] ${args?.firstOrNull()?.toString().orEmpty()}")
                    null
                }
                "onListeningHttpProxyPort" -> {
                    val port = (args?.firstOrNull() as? Number)?.toInt() ?: 0
                    if (port > 0) {
                        state.httpPort.set(port)
                        state.proxyReady.countDown()
                        log("[PSIPHON] Local HTTP proxy listening on 127.0.0.1:$port")
                    }
                    null
                }
                "onListeningSocksProxyPort" -> {
                    val port = (args?.firstOrNull() as? Number)?.toInt() ?: 0
                    if (port > 0) {
                        state.socksPort.set(port)
                        log("[PSIPHON] Local SOCKS proxy listening on 127.0.0.1:$port")
                    }
                    null
                }
                "onSocksProxyPortInUse", "onHttpProxyPortInUse" -> {
                    log("[PSIPHON] ${method.name}: ${args?.firstOrNull() ?: "unknown"}")
                    null
                }
                "onUpstreamProxyError" -> {
                    log("[PSIPHON] Upstream proxy error: ${args?.firstOrNull()?.toString().orEmpty()}")
                    null
                }
                "onConnecting" -> {
                    log("[PSIPHON] connecting")
                    null
                }
                "onConnected" -> {
                    state.connected.set(true)
                    log("[PSIPHON] connected")
                    null
                }
                "onClientRegion", "onConnectedServerRegion" -> {
                    log("[PSIPHON] ${method.name}: ${args?.firstOrNull()?.toString().orEmpty()}")
                    null
                }
                "onBytesTransferred" -> null
                "onStartedWaitingForNetworkConnectivity" -> {
                    log("[PSIPHON] waiting for network connectivity")
                    null
                }
                "onStoppedWaitingForNetworkConnectivity" -> {
                    log("[PSIPHON] network connectivity restored")
                    null
                }
                "onExiting" -> {
                    log("[PSIPHON] exiting")
                    null
                }
                "toString" -> "PacketPsiphonHostService"
                else -> defaultReturn(method.returnType)
            }
        }

        private fun defaultReturn(returnType: Class<*>): Any? {
            return when (returnType) {
                java.lang.Boolean.TYPE -> false
                java.lang.Integer.TYPE -> 0
                java.lang.Long.TYPE -> 0L
                java.lang.Float.TYPE -> 0f
                java.lang.Double.TYPE -> 0.0
                java.lang.Void.TYPE -> null
                else -> null
            }
        }
    }
}
