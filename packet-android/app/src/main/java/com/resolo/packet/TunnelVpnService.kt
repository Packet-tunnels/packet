package com.resolo.packet

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.pm.ServiceInfo
import android.content.Intent
import android.net.ConnectivityManager
import android.net.NetworkCapabilities
import android.net.Uri
import android.net.VpnService
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.os.ParcelFileDescriptor
import android.provider.Settings
import java.io.InputStream
import java.net.HttpURLConnection
import java.net.InetSocketAddress
import java.net.Socket
import java.net.URL
import java.text.SimpleDateFormat
import java.util.Date
import java.util.concurrent.FutureTask
import java.util.Locale
import java.util.TimeZone

class TunnelVpnService : VpnService() {
    private var vpnInterface: ParcelFileDescriptor? = null
    private var processExitScheduled = false
    private var activeDisconnectId = 0
    @Volatile private var connectInFlight = false
    private var activeConfiguration: TunnelConfiguration? = null
    private var lastRuntimeErrorLogged: String? = null
    private val processExitRunnable = Runnable {
        android.os.Process.killProcess(android.os.Process.myPid())
    }

    private val mainHandler = Handler(Looper.getMainLooper())
    private val telemetryRunnable = object : Runnable {
        override fun run() {
            refreshRuntimeTelemetry()
            mainHandler.postDelayed(this, TELEMETRY_REFRESH_MS)
        }
    }

    private val serviceLogCallback = object : PacketBridge.LogCallback {
        override fun onLog(message: String) {
            TunnelLogStore.append(applicationContext, message.trimEnd())
        }
    }

    override fun onCreate() {
        super.onCreate()
        ensureNotificationChannel()
        PacketBridge.setLogCallback(serviceLogCallback)
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        when (intent?.action ?: TunnelActions.ACTION_CONNECT) {
            TunnelActions.ACTION_DISCONNECT -> disconnectTunnel("Tunnel disconnected")
            TunnelActions.ACTION_CONNECT -> connectTunnel()
        }
        return START_NOT_STICKY
    }

    override fun onRevoke() {
        TunnelLogStore.append(this, "[VPN] Android revoked VPN permission")
        disconnectTunnel("VPN permission revoked")
    }

    override fun onDestroy() {
        stopTelemetryRefresh()
        closeInterface()
        super.onDestroy()
    }

    private fun connectTunnel() {
        cancelProcessExit()
        activeDisconnectId = 0
        if (connectInFlight) {
            TunnelLogStore.append(this, "[VPN] Connect request ignored because startup is already in progress")
            return
        }

        if (vpnInterface != null) {
            TunnelPreferences.updateState(this, TunnelState.RUNNING, "Tunnel already connected")
            updateNotification("Running")
            return
        }

        val configuration = TunnelPreferences.loadConfiguration(this)
        activeConfiguration = configuration
        TunnelPreferences.markSelectedConfigurationActive(this, configuration)
        lastRuntimeErrorLogged = null
        TunnelPreferences.updateRuntimeSnapshot(this, initialRuntimeSnapshot(configuration))
        TunnelPreferences.updateDiagnostics(this, initialDiagnostics(configuration))

        val validationError = validate(configuration)
        if (validationError != null) {
            failStart(validationError)
            return
        }

        connectInFlight = true
        startTunnelForeground("Connecting")
        TunnelPreferences.updateState(this, TunnelState.CONNECTING, "Starting Android VPN service")
        TunnelLogStore.append(this, "[VPN] Loading Android VPN service configuration")
        TunnelLogStore.append(this, "[VPN] Starting Android VPN service shell")
        runCatching {
            logStartupEvidence(configuration)
        }.onFailure { error ->
            TunnelLogStore.append(
                this,
                "[DIAG] Startup evidence collection failed: ${error.localizedMessage ?: error.javaClass.simpleName}",
            )
        }

        Thread {
            try {
                val preflightDiagnostics = performPreflightDiagnostics(configuration)
                TunnelPreferences.updateDiagnostics(this@TunnelVpnService, preflightDiagnostics)

                val listenPort = startRustCore(configuration)
                if (listenPort <= 0) {
                    mainHandler.post {
                        failStart(buildStartFailureMessage(listenPort, configuration))
                    }
                    return@Thread
                }

                val requestedPort = configuration.listenPortValue
                when {
                    requestedPort == null ->
                        TunnelLogStore.append(this@TunnelVpnService, "[VPN] Auto-selected local SOCKS5 port $listenPort")
                    requestedPort != listenPort ->
                        TunnelLogStore.append(
                            this@TunnelVpnService,
                            "[VPN] Requested local SOCKS5 port $requestedPort was busy, using $listenPort instead",
                        )
                    else ->
                        TunnelLogStore.append(this@TunnelVpnService, "[VPN] Using requested local SOCKS5 port $listenPort")
                }

                TunnelPreferences.updateRuntimeSnapshot(
                    this@TunnelVpnService,
                    TunnelPreferences.loadRuntimeSnapshot(this@TunnelVpnService).copy(listenPort = listenPort),
                )

                mainHandler.post {
                    startTelemetryRefresh()
                }

                if (!waitForLocalProxy(listenPort)) {
                    val runtime = TunnelPreferences.loadRuntimeSnapshot(this@TunnelVpnService)
                        .copy(listenPort = listenPort)
                    val diagnostics = TunnelPreferences.loadDiagnostics(this@TunnelVpnService).copy(
                        localProxyReady = false,
                        recommendation = buildRecommendation(
                            configuration = configuration,
                            diagnostics = TunnelPreferences.loadDiagnostics(this@TunnelVpnService),
                            lastError = "Local SOCKS5 listener did not answer on 127.0.0.1:$listenPort.",
                            runtime = runtime,
                        ),
                        lastFailureDetail = "Local SOCKS5 listener did not answer on 127.0.0.1:$listenPort.",
                        lastUpdatedMs = System.currentTimeMillis(),
                    )
                    TunnelPreferences.updateDiagnostics(this@TunnelVpnService, diagnostics)
                    mainHandler.post {
                        failStart("Local SOCKS5 listener was not ready on 127.0.0.1:$listenPort.")
                    }
                    return@Thread
                }

                TunnelLogStore.append(this@TunnelVpnService, "[DIAG] Local SOCKS5 listener is reachable on 127.0.0.1:$listenPort")
                val runtime = TunnelPreferences.loadRuntimeSnapshot(this@TunnelVpnService).copy(listenPort = listenPort)
                TunnelPreferences.updateDiagnostics(
                    this@TunnelVpnService,
                    TunnelPreferences.loadDiagnostics(this@TunnelVpnService).copy(
                        localProxyReady = true,
                        recommendation = buildRecommendation(
                            configuration = configuration,
                            diagnostics = TunnelPreferences.loadDiagnostics(this@TunnelVpnService).copy(localProxyReady = true),
                            lastError = null,
                            runtime = runtime,
                        ),
                        lastUpdatedMs = System.currentTimeMillis(),
                    ),
                )

                val egressProbe = waitForInternetEgress(configuration, listenPort)
                if (!egressProbe.succeeded) {
                    if (!connectInFlight) {
                        TunnelLogStore.append(this@TunnelVpnService, "[DIAG] Internet probe stopped because startup was cancelled")
                        return@Thread
                    }

                    val message = "Tunnel internet probe failed: ${egressProbe.detail}"
                    val failedRuntime = TunnelPreferences.loadRuntimeSnapshot(this@TunnelVpnService)
                        .copy(listenPort = listenPort, tunnelActive = false, lastError = message)
                    TunnelPreferences.updateRuntimeSnapshot(this@TunnelVpnService, failedRuntime)
                    TunnelPreferences.updateDiagnostics(
                        this@TunnelVpnService,
                        TunnelPreferences.loadDiagnostics(this@TunnelVpnService).copy(
                            recommendation = buildRecommendation(
                                configuration = configuration,
                                diagnostics = TunnelPreferences.loadDiagnostics(this@TunnelVpnService),
                                lastError = message,
                                runtime = failedRuntime,
                            ),
                            lastFailureDetail = message,
                            lastUpdatedMs = System.currentTimeMillis(),
                        ),
                    )
                    mainHandler.post {
                        failStart(message)
                    }
                    return@Thread
                }

                TunnelLogStore.append(
                    this@TunnelVpnService,
                    buildString {
                        append("[DIAG] Internet egress probe passed through local proxy via ${egressProbe.target}")
                        egressProbe.countryName?.let { append(" country=$it") }
                    },
                )

                if (!connectInFlight) {
                    TunnelLogStore.append(this@TunnelVpnService, "[DIAG] Startup cancelled before Android VPN interface creation")
                    return@Thread
                }

                mainHandler.post {
                    completeTunnelConnection(configuration, listenPort, egressProbe)
                }
            } catch (e: Exception) {
                mainHandler.post {
                    failStart(e.localizedMessage ?: "Failed to start tunnel.")
                }
            }
        }.start()
    }

    private fun completeTunnelConnection(
        configuration: TunnelConfiguration,
        listenPort: Int,
        egressProbe: EgressProbeResult,
    ) {
        val builder = createVpnBuilder()

        vpnInterface = try {
            builder.establish() ?: throw IllegalStateException("Android VPN permission is not available.")
        } catch (error: Exception) {
            failStart(error.localizedMessage ?: "Failed to establish Android VPN interface.")
            return
        }

        val tunBridgeResult = PacketBridge.startTun2Socks(
            requireNotNull(vpnInterface).fd,
            LOCAL_SOCKS_HOST,
            listenPort,
            VPN_MTU,
            VPN_DNS_SERVER,
        )
        if (tunBridgeResult != 0) {
            failStart("Failed to start the Android tun bridge (code $tunBridgeResult).")
            return
        }

        TunnelLogStore.append(this, "[VPN] Android VPN interface established")
        TunnelLogStore.append(
            this,
            "[DIAG] tun2socks bridge is forwarding device traffic into $LOCAL_SOCKS_HOST:$listenPort",
        )
        TunnelLogStore.append(
            this,
            "[DIAG] VPN DNS is mapped to $VPN_DNS_SERVER for hostname-aware TCP proxying",
        )
        TunnelLogStore.append(
            this,
            "[DIAG] UDP and Android Private DNS are not supported end-to-end yet; UDP-heavy apps may fail or retry repeatedly",
        )

        TunnelPreferences.updateDiagnostics(
            this,
            TunnelPreferences.loadDiagnostics(this).copy(
                vpnShellReady = true,
                recommendation = buildRecommendation(
                    configuration = configuration,
                    diagnostics = TunnelPreferences.loadDiagnostics(this).copy(
                        localProxyReady = true,
                        vpnShellReady = true,
                    ),
                    lastError = null,
                    runtime = TunnelPreferences.loadRuntimeSnapshot(this).copy(
                        listenPort = listenPort,
                        tunnelActive = true,
                    ),
                ),
                lastUpdatedMs = System.currentTimeMillis(),
            ),
        )

        TunnelPreferences.updateRuntimeSnapshot(
            this,
            TunnelPreferences.loadRuntimeSnapshot(this).copy(
                listenPort = listenPort,
                tunnelActive = true,
                egressPingMs = egressProbe.durationMs.toInt().takeIf { it > 0 },
                egressTarget = egressProbe.target,
                serverCountryCode = egressProbe.countryCode,
                serverCountryName = egressProbe.countryName,
            ),
        )

        TunnelPreferences.updateState(
            this,
            TunnelState.RUNNING,
            "Tunnel active — device traffic is routed through Android VPN and local SOCKS5 on port $listenPort",
        )
        connectInFlight = false
        updateNotification("Running")
        refreshRuntimeTelemetry()
    }

    private fun disconnectTunnel(message: String) {
        val disconnectId = activeDisconnectId + 1
        activeDisconnectId = disconnectId
        connectInFlight = false
        TunnelPreferences.updateState(this, TunnelState.DISCONNECTING, "Stopping Android VPN service")
        TunnelLogStore.append(this, "[VPN] Stop requested")
        stopTelemetryRefresh()
        Thread {
            closeInterface()
            mainHandler.post {
                finishDisconnect(disconnectId, message)
            }
        }.start()
        mainHandler.postDelayed({
            if (activeDisconnectId == disconnectId) {
                TunnelLogStore.append(this, "[VPN] Stop cleanup timed out; resetting tunnel process")
                finishDisconnect(disconnectId, message)
            }
        }, DISCONNECT_CLEANUP_TIMEOUT_MS)
    }

    private fun finishDisconnect(disconnectId: Int, message: String) {
        if (activeDisconnectId != disconnectId) {
            return
        }

        activeDisconnectId = 0
        stopForeground(STOP_FOREGROUND_REMOVE)

        TunnelPreferences.updateRuntimeSnapshot(
            this,
            TunnelPreferences.loadRuntimeSnapshot(this).copy(
                state = "idle",
                activeStreams = 0,
                connectedSince = null,
                lastPingMs = null,
                tunnelActive = false,
            ),
        )
        TunnelPreferences.updateDiagnostics(
            this,
            TunnelPreferences.loadDiagnostics(this).copy(
                vpnShellReady = false,
                localProxyReady = false,
                recommendation = buildRecommendation(
                    configuration = activeConfiguration
                        ?: TunnelPreferences.loadActiveConfiguration(this)
                        ?: TunnelPreferences.loadConfiguration(this),
                    diagnostics = TunnelPreferences.loadDiagnostics(this).copy(
                        vpnShellReady = false,
                        localProxyReady = false,
                    ),
                    lastError = null,
                ),
                lastUpdatedMs = System.currentTimeMillis(),
            ),
        )

        TunnelPreferences.updateState(this, TunnelState.IDLE, message)
        stopSelf()
        scheduleProcessExit()
    }

    private fun failStart(message: String) {
        connectInFlight = false
        val configuration = activeConfiguration
            ?: TunnelPreferences.loadActiveConfiguration(this)
            ?: TunnelPreferences.loadConfiguration(this)
        TunnelLogStore.append(this, "[VPN] $message")
        stopTelemetryRefresh()
        TunnelPreferences.updateRuntimeSnapshot(
            this,
            TunnelPreferences.loadRuntimeSnapshot(this).copy(
                state = "failed",
                connectedSince = null,
                tunnelActive = false,
                lastError = message,
            ),
        )
        TunnelPreferences.updateDiagnostics(
            this,
            TunnelPreferences.loadDiagnostics(this).copy(
                recommendation = buildRecommendation(
                    configuration = configuration,
                    diagnostics = TunnelPreferences.loadDiagnostics(this),
                    lastError = message,
                ),
                lastFailureDetail = message,
                lastUpdatedMs = System.currentTimeMillis(),
            ),
        )
        TunnelPreferences.updateState(this, TunnelState.FAILED, message)
        closeInterface()
        stopForeground(STOP_FOREGROUND_REMOVE)
        stopSelf()
        scheduleProcessExit()
    }

    private fun closeInterface() {
        runCatching { PacketBridge.stopTun2Socks() }
        runCatching { PacketBridge.stopClient() }
        runCatching { PacketBridge.stopLayeredCarrier() }
        runCatching { vpnInterface?.close() }
        vpnInterface = null
    }

    private fun createVpnBuilder(): Builder {
        val builder = Builder()
            .setSession("Packet")
            .setMtu(VPN_MTU)
            .addAddress(VPN_TUN_ADDRESS, VPN_TUN_PREFIX)
            .addDnsServer(VPN_DNS_SERVER)
            .addRoute("0.0.0.0", 0)

        runCatching {
            builder.addDisallowedApplication(packageName)
        }.getOrElse { error ->
            throw IllegalStateException(
                "Failed to exclude $packageName from the VPN: ${error.localizedMessage ?: error.javaClass.simpleName}",
            )
        }

        TunnelLogStore.append(
            this,
            "[VPN] Routing IPv4 traffic through Android VpnService and excluding $packageName to avoid tunnel loops",
        )
        return builder
    }

    private fun scheduleProcessExit() {
        if (processExitScheduled) {
            return
        }

        processExitScheduled = true
        mainHandler.postDelayed(processExitRunnable, 250)
    }

    private fun cancelProcessExit() {
        if (!processExitScheduled) {
            return
        }

        mainHandler.removeCallbacks(processExitRunnable)
        processExitScheduled = false
    }

    private fun startTelemetryRefresh() {
        mainHandler.removeCallbacks(telemetryRunnable)
        mainHandler.post(telemetryRunnable)
    }

    private fun stopTelemetryRefresh() {
        mainHandler.removeCallbacks(telemetryRunnable)
    }

    private fun refreshRuntimeTelemetry() {
        val configuration = activeConfiguration
            ?: TunnelPreferences.loadActiveConfiguration(this)
            ?: TunnelPreferences.loadConfiguration(this)
        val previousRuntime = TunnelPreferences.loadRuntimeSnapshot(this)
        val runtimeSnapshot = mergeRuntimeProbeMetadata(
            TunnelRuntimeSnapshot.fromJsonString(PacketBridge.copyStatsJson()),
            previousRuntime,
        )
        TunnelPreferences.updateRuntimeSnapshot(this, runtimeSnapshot)

        if (!runtimeSnapshot.lastError.isNullOrBlank() && runtimeSnapshot.lastError != lastRuntimeErrorLogged) {
            lastRuntimeErrorLogged = runtimeSnapshot.lastError
            TunnelLogStore.append(this, "[DIAG] Runtime error: ${runtimeSnapshot.lastError}")
        }

        val diagnostics = TunnelPreferences.loadDiagnostics(this)
        TunnelPreferences.updateDiagnostics(
            this,
            diagnostics.copy(
                recommendation = buildRecommendation(
                    configuration = configuration,
                    diagnostics = diagnostics,
                    lastError = runtimeSnapshot.lastError,
                    runtime = runtimeSnapshot,
                ),
                lastFailureDetail = runtimeSnapshot.lastError ?: diagnostics.lastFailureDetail,
                lastUpdatedMs = System.currentTimeMillis(),
            ),
        )
    }

    private fun mergeRuntimeProbeMetadata(
        runtimeSnapshot: TunnelRuntimeSnapshot,
        previousRuntime: TunnelRuntimeSnapshot,
    ): TunnelRuntimeSnapshot {
        if (!runtimeSnapshot.tunnelActive) {
            return runtimeSnapshot
        }

        return runtimeSnapshot.copy(
            serverCountryCode = runtimeSnapshot.serverCountryCode ?: previousRuntime.serverCountryCode,
            serverCountryName = runtimeSnapshot.serverCountryName ?: previousRuntime.serverCountryName,
            egressPingMs = runtimeSnapshot.egressPingMs ?: previousRuntime.egressPingMs,
            egressTarget = runtimeSnapshot.egressTarget ?: previousRuntime.egressTarget,
        )
    }

    private fun startRustCore(configuration: TunnelConfiguration): Int {
        // Iran cellular carriers (IranCell, etc.) advertise an HTTP proxy on
        // the APN that foreign-IP traffic MUST traverse to escape the
        // network blackhole. Detect it here and chain the Trojan carrier
        // through it via `upstream_http=`. Operator-set upstreams in the URI
        // are preserved unchanged.
        val systemProxy = SystemProxyDetector.detect(this)
        val effectiveTrojanUri = run {
            val base = configuration.normalizedTrojanCarrierUri
            if (systemProxy != null) {
                val rewritten = SystemProxyDetector.appendToTrojanUri(base, systemProxy)
                if (rewritten != base) {
                    TunnelLogStore.append(
                        this,
                        "[CHAIN] System HTTP proxy detected ($systemProxy) — routing Trojan carrier via it",
                    )
                }
                rewritten
            } else {
                base
            }
        }

        if (configuration.usesCustomCarrier) {
            return PacketBridge.startLayeredCarrierFull(
                effectiveTrojanUri,
                configuration.carrierProxyPortValue,
                configuration.fragmentEnabled,
                configuration.fragmentSizeValue,
            )
        }

        if (configuration.usesPacketChain) {
            val carrierPort = PacketBridge.startLayeredCarrierFull(
                effectiveTrojanUri,
                configuration.carrierProxyPortValue,
                configuration.fragmentEnabled,
                configuration.fragmentSizeValue,
            )
            if (carrierPort <= 0) {
                TunnelLogStore.append(
                    this,
                    "[CHAIN] DirectSock carrier failed to start before Packet escape layer (code $carrierPort)",
                )
                return carrierPort
            }

            val upstreamProxy = "http://127.0.0.1:$carrierPort"
            TunnelLogStore.append(
                this,
                "[CHAIN] DirectSock carrier is listening on 127.0.0.1:$carrierPort; starting Packet Meek HTTP through it",
            )

            return PacketBridge.startClientFull(
                configuration.normalizedServerUrl,
                configuration.normalizedSecret,
                configuration.listenPortValue ?: 0,
                configuration.normalizedCdnEdge,
                configuration.normalizedHostOverride,
                configuration.normalizedSniOverride,
                // Hand the configured transport mode straight through; the
                // chain default is AUTO so the Rust rotation supervisor
                // (run_rotating_transport) sweeps WS / Obfs / QUIC across
                // ports until something punches through. Hardcoding MEEK
                // here would lock us to a single shape that Iran RSTs.
                configuration.transportMode.rawValue,
                configuration.fragmentEnabled,
                configuration.fragmentSizeValue,
                configuration.normalizedObfsKey,
                upstreamProxy,
            )
        }

        val listenPort = configuration.listenPortValue ?: 0
        if (configuration.usesPrivateRelay) {
            TunnelLogStore.append(
                this,
                "[PRIVATE] Starting private relay client: single WebSocket lane, decoy=off, padding=off",
            )
            return PacketBridge.startClientPrivateRelay(
                configuration.normalizedServerUrl,
                configuration.normalizedSecret,
                listenPort,
            )
        }

        return if (configuration.usesAdvancedStart) {
            PacketBridge.startClientFull(
                configuration.normalizedServerUrl,
                configuration.normalizedSecret,
                listenPort,
                configuration.normalizedCdnEdge,
                configuration.normalizedHostOverride,
                configuration.normalizedSniOverride,
                configuration.transportMode.rawValue,
                configuration.fragmentEnabled,
                configuration.fragmentSizeValue,
                configuration.normalizedObfsKey,
                configuration.normalizedUpstreamProxy,
            )
        } else {
            PacketBridge.startClient(
                configuration.normalizedServerUrl,
                configuration.normalizedSecret,
                listenPort,
            )
        }
    }

    private fun waitForLocalProxy(listenPort: Int): Boolean {
        val deadline = System.currentTimeMillis() + 5_000
        var attempts = 0

        while (System.currentTimeMillis() < deadline) {
            attempts += 1
            if (canConnectToLocalProxy(listenPort)) {
                TunnelLogStore.append(this, "[DIAG] Local proxy probe succeeded after $attempts attempt(s)")
                return true
            }

            if (attempts % 5 == 0) {
                TunnelLogStore.append(this, "[DIAG] Waiting for local SOCKS5 on 127.0.0.1:$listenPort ($attempts checks)")
            }

            Thread.sleep(200)
        }

        return false
    }

    private fun canConnectToLocalProxy(listenPort: Int): Boolean {
        return runCatching {
            Socket().use { socket ->
                socket.connect(InetSocketAddress("127.0.0.1", listenPort), 250)
                true
            }
        }.getOrDefault(false)
    }

    private fun waitForInternetEgress(
        configuration: TunnelConfiguration,
        listenPort: Int,
    ): EgressProbeResult {
        val timeoutMs = if (configuration.usesCustomCarrier) {
            CARRIER_EGRESS_TIMEOUT_MS
        } else if (configuration.usesPacketChain) {
            CARRIER_EGRESS_TIMEOUT_MS
        } else if (configuration.usesPrivateRelay) {
            CARRIER_EGRESS_TIMEOUT_MS
        } else {
            STANDARD_EGRESS_TIMEOUT_MS
        }
        val startedAt = System.currentTimeMillis()
        val deadline = startedAt + timeoutMs
        val label = when {
            configuration.usesPrivateRelay -> "Private Relay"
            configuration.usesPacketChain -> "Packet Chain"
            configuration.usesCustomCarrier -> "DirectSock"
            else -> "tunnel"
        }
        var attempts = 0
        var lastProbe = EgressProbeResult(
            succeeded = false,
            target = "none",
            detail = "Internet probe did not run.",
            durationMs = 0,
            countryCode = null,
            countryName = null,
        )

        TunnelPreferences.updateState(
            this,
            TunnelState.CONNECTING,
            "Waiting for $label internet access",
        )
        updateNotification("Testing network access")
        TunnelLogStore.append(
            this,
            "[DIAG] Waiting up to ${timeoutMs / 1000}s for real internet egress through 127.0.0.1:$listenPort",
        )

        while (System.currentTimeMillis() < deadline) {
            if (!connectInFlight) {
                return lastProbe.copy(detail = "Startup was cancelled.")
            }

            attempts += 1
            lastProbe = probeInternetThroughSocks(listenPort)
            if (lastProbe.succeeded) {
                return lastProbe
            }

            val elapsedSeconds = (System.currentTimeMillis() - startedAt) / 1000
            val remainingSeconds = ((deadline - System.currentTimeMillis()).coerceAtLeast(0)) / 1000
            if (attempts == 1 || attempts % 3 == 0) {
                TunnelLogStore.append(
                    this,
                    "[DIAG] $label internet probe still waiting after ${elapsedSeconds}s: ${lastProbe.detail}",
                )
            }
            TunnelPreferences.updateState(
                this,
                TunnelState.CONNECTING,
                "Waiting for $label internet access (${remainingSeconds}s left)",
            )
            TunnelPreferences.updateDiagnostics(
                this,
                TunnelPreferences.loadDiagnostics(this).copy(
                    localProxyReady = true,
                    vpnShellReady = false,
                    healthStatus = "Internet probe pending",
                    lastFailureDetail = lastProbe.detail,
                    lastUpdatedMs = System.currentTimeMillis(),
                ),
            )

            Thread.sleep(EGRESS_RETRY_DELAY_MS)
        }

        return lastProbe.copy(
            detail = "No HTTP response through 127.0.0.1:$listenPort within ${timeoutMs / 1000}s. Last error: ${lastProbe.detail}",
        )
    }

    private fun probeInternetThroughSocks(listenPort: Int): EgressProbeResult {
        val failures = mutableListOf<String>()
        var firstSuccessfulProbe: EgressProbeResult? = null

        for (target in EGRESS_PROBE_TARGETS) {
            val startedAt = System.currentTimeMillis()
            try {
                Socket().use { socket ->
                    socket.soTimeout = EGRESS_SOCKET_TIMEOUT_MS
                    socket.connect(InetSocketAddress("127.0.0.1", listenPort), EGRESS_CONNECT_TIMEOUT_MS)

                    val input = socket.getInputStream()
                    val output = socket.getOutputStream()

                    output.write(byteArrayOf(0x05.toByte(), 0x01.toByte(), 0x00.toByte()))
                    output.flush()

                    val authReply = readExact(input, 2)
                    if (authReply[0].toInt() != 0x05 || authReply[1].toInt() != 0x00) {
                        throw IllegalStateException("SOCKS5 auth failed")
                    }

                    val hostBytes = target.host.toByteArray(Charsets.US_ASCII)
                    val request = ByteArray(7 + hostBytes.size)
                    request[0] = 0x05.toByte()
                    request[1] = 0x01.toByte()
                    request[2] = 0x00.toByte()
                    request[3] = 0x03.toByte()
                    request[4] = hostBytes.size.toByte()
                    System.arraycopy(hostBytes, 0, request, 5, hostBytes.size)
                    request[5 + hostBytes.size] = ((target.port shr 8) and 0xFF).toByte()
                    request[6 + hostBytes.size] = (target.port and 0xFF).toByte()

                    output.write(request)
                    output.flush()

                    val connectReply = readExact(input, 4)
                    if (connectReply[0].toInt() != 0x05) {
                        throw IllegalStateException("Invalid SOCKS5 reply")
                    }
                    if (connectReply[1].toInt() != 0x00) {
                        throw IllegalStateException(
                            "SOCKS5 CONNECT failed: ${socksReplyCodeLabel(connectReply[1].toInt() and 0xFF)}",
                        )
                    }
                    consumeSocksBindAddress(input, connectReply[3].toInt() and 0xFF)

                    output.write(target.request.toByteArray(Charsets.US_ASCII))
                    output.flush()

                    val responseBuffer = ByteArray(1024)
                    val bytesRead = input.read(responseBuffer)
                    if (bytesRead <= 0) {
                        throw IllegalStateException("CONNECT succeeded but no HTTP bytes returned")
                    }

                    val rawResponse = String(responseBuffer, 0, bytesRead, Charsets.US_ASCII)
                    val preview = rawResponse
                        .replace('\n', ' ')
                        .replace('\r', ' ')
                    if (!preview.startsWith("HTTP/")) {
                        throw IllegalStateException("Non-HTTP response: ${preview.take(80)}")
                    }

                    val country = parseProbeCountry(target.host, rawResponse)
                    val result = EgressProbeResult(
                        succeeded = true,
                        target = "${target.host}:${target.port}",
                        detail = preview.take(80),
                        durationMs = System.currentTimeMillis() - startedAt,
                        countryCode = country.code,
                        countryName = country.name,
                    )
                    if (result.countryCode != null || result.countryName != null) {
                        return result
                    }
                    if (firstSuccessfulProbe == null) {
                        firstSuccessfulProbe = result
                    }
                }
            } catch (error: Exception) {
                failures += "${target.host}:${target.port} -> ${error.localizedMessage ?: error.javaClass.simpleName}"
            }
        }

        firstSuccessfulProbe?.let { return it }

        return EgressProbeResult(
            succeeded = false,
            target = EGRESS_PROBE_TARGETS.joinToString(", ") { "${it.host}:${it.port}" },
            detail = failures.joinToString(" | "),
            durationMs = 0,
            countryCode = null,
            countryName = null,
        )
    }

    private fun parseProbeCountry(host: String, rawResponse: String): ProbeCountry {
        parseTraceCountryCode(rawResponse)?.let { code ->
            return ProbeCountry(
                code = code,
                name = Locale("", code).displayCountry,
            )
        }

        if (!host.equals("ip-api.com", ignoreCase = true)) {
            return ProbeCountry(null, null)
        }

        val body = rawResponse.substringAfter("\r\n\r\n", rawResponse)
        val lines = body.lines()
            .map { it.trim() }
            .filter { it.isNotEmpty() }
        val codeIndex = lines.indexOfFirst { line ->
            line.length == 2 && line.all { it.isLetter() } && line.uppercase(Locale.US) == line
        }
        if (codeIndex < 0) {
            return ProbeCountry(null, null)
        }

        val code = lines[codeIndex].uppercase(Locale.US)
        val name = lines.getOrNull(codeIndex - 1)
            ?.takeIf { it.length > 2 && it.any(Char::isLetter) }
            ?: Locale("", code).displayCountry
        return ProbeCountry(code, name)
    }

    private fun parseTraceCountryCode(preview: String): String? {
        val marker = "loc="
        val start = preview.indexOf(marker)
        if (start < 0) {
            return null
        }

        val code = preview.drop(start + marker.length)
            .takeWhile { it.isLetter() }
            .uppercase(Locale.US)
        return code.takeIf { it.length == 2 }
    }

    private fun readExact(input: InputStream, byteCount: Int): ByteArray {
        val buffer = ByteArray(byteCount)
        var offset = 0
        while (offset < byteCount) {
            val read = input.read(buffer, offset, byteCount - offset)
            if (read < 0) {
                throw IllegalStateException("Expected $byteCount bytes, got $offset")
            }
            offset += read
        }
        return buffer
    }

    private fun consumeSocksBindAddress(input: InputStream, addressType: Int) {
        when (addressType) {
            0x01 -> readExact(input, 6)
            0x03 -> {
                val hostLength = readExact(input, 1)[0].toInt() and 0xFF
                readExact(input, hostLength + 2)
            }
            0x04 -> readExact(input, 18)
            else -> throw IllegalStateException("Unsupported SOCKS5 bind address type $addressType")
        }
    }

    private fun socksReplyCodeLabel(code: Int): String {
        return when (code) {
            0x01 -> "general failure"
            0x02 -> "connection not allowed"
            0x03 -> "network unreachable"
            0x04 -> "host unreachable"
            0x05 -> "connection refused"
            0x06 -> "TTL expired"
            0x07 -> "command not supported"
            0x08 -> "address type not supported"
            else -> "code $code"
        }
    }

    private fun performPreflightDiagnostics(configuration: TunnelConfiguration): TunnelDiagnosticsSnapshot {
        val endpointProbe = runOnWorkerThread { probeEndpoint(configuration.endpointHost, configuration.endpointPort) }
        val healthProbe = if (configuration.usesPacketChain) {
            HealthProbe(
                status = "Health skipped for Packet Chain",
                error = "Direct Packet server health is intentionally skipped; the Packet escape layer dials through the local DirectSock carrier.",
            )
        } else {
            runOnWorkerThread { probeHealth(configuration.normalizedServerUrl) }
        }

        val diagnostics = TunnelDiagnosticsSnapshot(
            endpointHost = configuration.endpointHost,
            endpointReachable = endpointProbe.reachable,
            endpointLatencyMs = endpointProbe.latencyMs,
            healthStatus = healthProbe.status,
            localProxyReady = false,
            vpnShellReady = false,
            routingComparison = platformRoutingComparison(),
            recommendation = buildRecommendation(
                configuration = configuration,
                diagnostics = TunnelDiagnosticsSnapshot(
                    endpointHost = configuration.endpointHost,
                    endpointReachable = endpointProbe.reachable,
                    endpointLatencyMs = endpointProbe.latencyMs,
                    healthStatus = healthProbe.status,
                    localProxyReady = false,
                    vpnShellReady = false,
                    routingComparison = platformRoutingComparison(),
                    recommendation = "",
                    lastFailureDetail = endpointProbe.error ?: healthProbe.error,
                    lastUpdatedMs = System.currentTimeMillis(),
                ),
                lastError = endpointProbe.error ?: healthProbe.error,
            ),
            lastFailureDetail = endpointProbe.error ?: healthProbe.error,
            lastUpdatedMs = System.currentTimeMillis(),
        )

        val probeSummary = if (endpointProbe.reachable == true) {
            "reachable in ${endpointProbe.latencyMs ?: 0} ms"
        } else {
            endpointProbe.error ?: "unreachable"
        }
        TunnelLogStore.append(
            this,
            "[DIAG] Endpoint probe ${configuration.endpointHost}:${configuration.endpointPort} -> $probeSummary",
        )
        TunnelLogStore.append(this, "[DIAG] Health probe -> ${healthProbe.status}")
        TunnelLogStore.append(this, "[DIAG] ${diagnostics.routingComparison}")
        TunnelLogStore.append(this, "[DIAG] Recommendation: ${diagnostics.recommendation}")
        return diagnostics
    }

    private fun logStartupEvidence(configuration: TunnelConfiguration) {
        if (configuration.usesCustomCarrier) {
            TunnelLogStore.append(
                this,
                "[DIAG] Config summary: stack=directsock_trojan endpoint=${configuration.endpointHost}:${configuration.endpointPort} " +
                    "local_proxy=${configuration.carrierProxyPortValue}",
            )
            TunnelLogStore.append(this, "[DIAG] Device clock: ${deviceClockSummary()}")
            TunnelLogStore.append(this, "[DIAG] Active network: ${activeNetworkSummary()}")
            TunnelLogStore.append(this, "[DIAG] Android Private DNS: ${privateDnsSummary()}")
            return
        }

        if (configuration.usesPacketChain) {
            TunnelLogStore.append(
                this,
                "[DIAG] Config summary: stack=packet_chain carrier=${configuration.endpointHost}:${configuration.endpointPort} " +
                    "carrier_local_port=${configuration.carrierProxyPortValue} packet_edge=${configuration.normalizedCdnEdge} " +
                    "packet_server=${configuration.normalizedServerUrl} fragment=${configuration.fragmentEnabled} " +
                    "fragment_size=${configuration.fragmentSizeValue} secret=${redactSecret(configuration.normalizedSecret)}",
            )
            TunnelLogStore.append(this, "[DIAG] Chain flow: app VPN -> Packet WebSocket SOCKS -> local DirectSock HTTP CONNECT -> Trojan/Cloudflare carrier -> Packet server")
            TunnelLogStore.append(this, "[DIAG] Device clock: ${deviceClockSummary()}")
            TunnelLogStore.append(this, "[DIAG] Active network: ${activeNetworkSummary()}")
            TunnelLogStore.append(this, "[DIAG] Android Private DNS: ${privateDnsSummary()}")
            return
        }

        if (configuration.usesPrivateRelay) {
            TunnelLogStore.append(
                this,
                "[DIAG] Config summary: stack=private_relay vps=${configuration.normalizedServerUrl} " +
                    "listen_port=${configuration.listenPort.ifBlank { "auto" }} secret=${redactSecret(configuration.normalizedSecret)}",
            )
            TunnelLogStore.append(this, "[DIAG] Chain flow: app VPN -> private Iran VPS -> authenticated reverse Starlink relay -> internet")
            TunnelLogStore.append(this, "[DIAG] Risk controls: public Trojan=off, decoy=off, padding=off, multi-lane=off")
            TunnelLogStore.append(this, "[DIAG] Device clock: ${deviceClockSummary()}")
            TunnelLogStore.append(this, "[DIAG] Active network: ${activeNetworkSummary()}")
            TunnelLogStore.append(this, "[DIAG] Android Private DNS: ${privateDnsSummary()}")
            return
        }

        TunnelLogStore.append(
            this,
            "[DIAG] Config summary: server_url=${configuration.normalizedServerUrl} server_host=${configuration.serverHost} " +
                "transport=${configuration.transportLabel} listen_port=${configuration.listenPort.ifBlank { "auto" }} " +
                "uses_cdn=${configuration.usesCdn} cdn_edge=${configuration.normalizedCdnEdge.ifBlank { "(empty)" }} " +
                "host_override=${configuration.normalizedHostOverride.ifBlank { "(empty)" }} " +
                "sni_override=${configuration.normalizedSniOverride.ifBlank { "(empty)" }} " +
                "upstream_proxy=${redactProxy(configuration.normalizedUpstreamProxy).ifBlank { "(empty)" }} " +
                "secret=${redactSecret(configuration.normalizedSecret)}",
        )
        TunnelLogStore.append(this, "[DIAG] Device clock: ${deviceClockSummary()}")
        TunnelLogStore.append(this, "[DIAG] Active network: ${activeNetworkSummary()}")
        TunnelLogStore.append(this, "[DIAG] Android Private DNS: ${privateDnsSummary()}")
    }

    private fun buildStartFailureMessage(resultCode: Int, configuration: TunnelConfiguration): String {
        return when (resultCode) {
            -2 -> "Local SOCKS5 port ${configuration.listenPort} is already in use. A stale process may still be holding it."
            else -> "Rust tunnel core failed to start with code $resultCode."
        }
    }

    private fun buildRecommendation(
        configuration: TunnelConfiguration,
        diagnostics: TunnelDiagnosticsSnapshot,
        lastError: String?,
        runtime: TunnelRuntimeSnapshot? = null,
    ): String {
        val activeRuntime = runtime ?: TunnelPreferences.loadRuntimeSnapshot(this)
        val forwardingPort = activeRuntime.listenPort ?: configuration.listenPortValue
        val parts = mutableListOf<String>()

        if (lastError != null) {
            parts.add("Last Error: $lastError")
        }

        when (diagnostics.endpointReachable) {
            true -> parts.add("Endpoint reachable (${diagnostics.endpointLatencyMs}ms).")
            false -> {
                val routeHint = if (configuration.usesPacketChain) {
                    "The Trojan/Cloudflare carrier is not reachable, so the Packet escape layer cannot bootstrap."
                } else if (configuration.usesCdn) {
                    "Check the reachable ingress: CDN edge, domestic bridge, host override, SNI override, or upstream origin."
                } else {
                    "The configured endpoint is not reachable from this network. In Iran, direct foreign endpoints often fail; use a domestic bridge or a fronted CDN edge."
                }
                parts.add("Endpoint unreachable. $routeHint")
            }
            null -> Unit
        }

        if (diagnostics.healthStatus.startsWith("Health HTTP")) {
            parts.add("Server health check passed.")
        } else if (diagnostics.healthStatus.isNotBlank() && diagnostics.healthStatus != "Not checked") {
            parts.add("Server health check failed: ${diagnostics.healthStatus}.")
        }

        if (diagnostics.localProxyReady && diagnostics.vpnShellReady) {
            parts.add(
                if (configuration.usesPacketChain) {
                    "Full-device forwarding is active. Device traffic is routed through Packet Chain on $LOCAL_SOCKS_HOST:${forwardingPort ?: "auto"}."
                } else if (configuration.usesCustomCarrier) {
                    "Full-device forwarding is active. Device traffic is routed through tun2socks into DirectSock on $LOCAL_SOCKS_HOST:${forwardingPort ?: "auto"}."
                } else {
                    "Full-device forwarding is active. Device traffic is routed through tun2socks into $LOCAL_SOCKS_HOST:${forwardingPort ?: "auto"}."
                }
            )
            parts.add("UDP and Android Private DNS are not supported end-to-end yet, so some apps may fail or retry.")
        } else if (diagnostics.localProxyReady) {
            parts.add("Local SOCKS5 proxy is ready on $LOCAL_SOCKS_HOST:${forwardingPort ?: "auto"}, but the Android tun bridge is not up yet.")
        }

        return parts.joinToString(" ")
    }

    private fun platformRoutingComparison(): String {
        return "Android routing: VpnService interface + tun2socks bridge + local SOCKS5 upstream."
    }

    private fun initialRuntimeSnapshot(configuration: TunnelConfiguration): TunnelRuntimeSnapshot {
        return TunnelRuntimeSnapshot(
            state = "starting",
            transport = if (configuration.usesCustomCarrier || configuration.usesPacketChain || configuration.usesPrivateRelay) configuration.ingressLabel else configuration.transportLabel,
            serverHost = configuration.serverHost,
            cdnEdge = configuration.normalizedCdnEdge.takeIf { it.isNotBlank() },
            tunnelActive = false,
        )
    }

    private fun initialDiagnostics(configuration: TunnelConfiguration): TunnelDiagnosticsSnapshot {
        return TunnelDiagnosticsSnapshot(
            endpointHost = configuration.endpointHost,
            routingComparison = platformRoutingComparison(),
            recommendation = buildRecommendation(
                configuration = configuration,
                diagnostics = TunnelDiagnosticsSnapshot(
                    endpointHost = configuration.endpointHost,
                    routingComparison = platformRoutingComparison(),
                ),
                lastError = null,
            ),
            lastUpdatedMs = System.currentTimeMillis(),
        )
    }

    private fun validate(configuration: TunnelConfiguration): String? {
        configuration.validationError?.let { return it }

        if (configuration.usesCustomCarrier) {
            return null
        }

        if (configuration.normalizedServerUrl.isEmpty()) {
            return "Server URL is required."
        }

        if (configuration.normalizedSecret.isEmpty()) {
            return "Shared secret is required."
        }

        val trimmedPort = configuration.listenPort.trim()
        if (trimmedPort.isNotEmpty() &&
            !trimmedPort.equals("auto", ignoreCase = true) &&
            configuration.listenPortValue == null
        ) {
            return "Listen port must be 1024-65535, or leave it blank for auto."
        }

        configuration.cdnEdgeValidationError?.let { return it }

        if (configuration.normalizedSniOverride.isNotEmpty() &&
            !configuration.normalizedServerUrl.startsWith("https://", ignoreCase = true)
        ) {
            return "SNI override requires an https:// server URL."
        }

        return null
    }

    private fun ensureNotificationChannel() {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) {
            return
        }

        val manager = getSystemService(NotificationManager::class.java)
        val channel = NotificationChannel(
            NOTIFICATION_CHANNEL_ID,
            "Packet",
            NotificationManager.IMPORTANCE_LOW,
        )
        channel.description = "Packet connection status"
        manager.createNotificationChannel(channel)
    }

    private fun updateNotification(status: String) {
        val manager = getSystemService(NotificationManager::class.java)
        manager.notify(NOTIFICATION_ID, buildNotification(status))
    }

    private fun startTunnelForeground(status: String) {
        val notification = buildNotification(status)
        if (Build.VERSION.SDK_INT >= 34) {
            startForeground(
                NOTIFICATION_ID,
                notification,
                ServiceInfo.FOREGROUND_SERVICE_TYPE_SYSTEM_EXEMPTED,
            )
        } else {
            startForeground(NOTIFICATION_ID, notification)
        }
    }

    private fun buildNotification(status: String): Notification {
        val launchIntent = Intent(this, MainActivity::class.java)
        val flags = PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
        val pendingIntent = PendingIntent.getActivity(this, 0, launchIntent, flags)
        val builder = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            Notification.Builder(this, NOTIFICATION_CHANNEL_ID)
        } else {
            Notification.Builder(this)
        }

        return builder
            .setContentTitle("Packet")
            .setContentText(status)
            .setSmallIcon(android.R.drawable.stat_sys_download_done)
            .setOngoing(true)
            .setContentIntent(pendingIntent)
            .build()
    }

    private fun probeEndpoint(host: String, port: Int): EndpointProbe {
        if (host.isBlank()) {
            return EndpointProbe(reachable = null, latencyMs = null, error = "Endpoint host is missing.")
        }

        return runCatching {
            val startedAt = System.currentTimeMillis()
            Socket().use { socket ->
                socket.connect(InetSocketAddress(host, port), 2_500)
            }
            EndpointProbe(
                reachable = true,
                latencyMs = (System.currentTimeMillis() - startedAt).toInt(),
                error = null,
            )
        }.getOrElse { error ->
            EndpointProbe(
                reachable = false,
                latencyMs = null,
                error = error.localizedMessage ?: "TCP probe failed",
            )
        }
    }

    private fun probeHealth(serverUrl: String): HealthProbe {
        if (serverUrl.isBlank()) {
            return HealthProbe("Health skipped", "Server URL is empty.")
        }

        return runCatching {
            val healthUrl = URL("${serverUrl.trimEnd('/')}/api/v1/health")
            val connection = (healthUrl.openConnection() as HttpURLConnection).apply {
                requestMethod = "GET"
                connectTimeout = 3_000
                readTimeout = 3_000
                instanceFollowRedirects = true
            }

            try {
                val statusCode = connection.responseCode
                HealthProbe(status = "Health HTTP $statusCode", error = null)
            } finally {
                connection.disconnect()
            }
        }.getOrElse { error ->
            HealthProbe(
                status = "Health request failed",
                error = error.localizedMessage ?: "Unknown health probe failure",
            )
        }
    }

    private fun <T> runOnWorkerThread(block: () -> T): T {
        val task = FutureTask(block)
        Thread(task).start()
        return task.get()
    }

    private fun deviceClockSummary(): String {
        val nowMs = System.currentTimeMillis()
        val utc = SimpleDateFormat("yyyy-MM-dd'T'HH:mm:ss.SSS'Z'", Locale.US).apply {
            timeZone = TimeZone.getTimeZone("UTC")
        }.format(Date(nowMs))
        return "utc=$utc epoch_ms=$nowMs timezone=${TimeZone.getDefault().id}"
    }

    private fun activeNetworkSummary(): String {
        return runCatching {
            val connectivity = getSystemService(ConnectivityManager::class.java)
                ?: return "ConnectivityManager unavailable"
            val network = connectivity.activeNetwork ?: return "No active network"
            val caps = connectivity.getNetworkCapabilities(network)
                ?: return "No network capabilities"

            val transports = buildList {
                if (caps.hasTransport(NetworkCapabilities.TRANSPORT_WIFI)) add("WIFI")
                if (caps.hasTransport(NetworkCapabilities.TRANSPORT_CELLULAR)) add("CELLULAR")
                if (caps.hasTransport(NetworkCapabilities.TRANSPORT_ETHERNET)) add("ETHERNET")
                if (caps.hasTransport(NetworkCapabilities.TRANSPORT_VPN)) add("VPN")
                if (caps.hasTransport(NetworkCapabilities.TRANSPORT_BLUETOOTH)) add("BLUETOOTH")
            }.ifEmpty { listOf("UNKNOWN") }

            "transports=${transports.joinToString("+")} validated=${caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_VALIDATED)} " +
                "internet=${caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)} " +
                "not_metered=${caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_NOT_METERED)} " +
                "captive_portal=${caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_CAPTIVE_PORTAL)}"
        }.getOrElse { error ->
            "Unavailable: ${error.localizedMessage ?: error.javaClass.simpleName}"
        }
    }

    private fun privateDnsSummary(): String {
        return runCatching {
            val mode = Settings.Global.getString(contentResolver, "private_dns_mode") ?: "unset"
            val specifier = Settings.Global.getString(contentResolver, "private_dns_specifier").orEmpty()
            if (specifier.isBlank()) {
                mode
            } else {
                "$mode ($specifier)"
            }
        }.getOrElse { error ->
            "Unavailable: ${error.localizedMessage ?: error.javaClass.simpleName}"
        }
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

    private fun redactProxy(proxy: String): String {
        if (proxy.isBlank()) {
            return ""
        }

        return runCatching {
            val uri = Uri.parse(proxy)
            val scheme = uri.scheme ?: return proxy
            val host = uri.host ?: return proxy
            val port = if (uri.port > 0) ":${uri.port}" else ""
            val userInfo = uri.userInfo?.takeIf { it.isNotBlank() }?.let { "user:redacted@" } ?: ""
            "$scheme://$userInfo$host$port"
        }.getOrDefault("(invalid)")
    }

    private data class EndpointProbe(
        val reachable: Boolean?,
        val latencyMs: Int?,
        val error: String?,
    )

    private data class HealthProbe(
        val status: String,
        val error: String?,
    )

    private data class EgressProbeTarget(
        val host: String,
        val port: Int,
        val request: String,
    )

    private data class EgressProbeResult(
        val succeeded: Boolean,
        val target: String,
        val detail: String,
        val durationMs: Long,
        val countryCode: String?,
        val countryName: String?,
    )

    private data class ProbeCountry(
        val code: String?,
        val name: String?,
    )

    private companion object {
        const val NOTIFICATION_CHANNEL_ID = "packet_status"
        const val NOTIFICATION_ID = 1001
        const val TELEMETRY_REFRESH_MS = 1_000L
        const val CARRIER_EGRESS_TIMEOUT_MS = 300_000L
        const val STANDARD_EGRESS_TIMEOUT_MS = 300_000L
        const val EGRESS_RETRY_DELAY_MS = 2_000L
        const val DISCONNECT_CLEANUP_TIMEOUT_MS = 3_500L
        const val EGRESS_CONNECT_TIMEOUT_MS = 1_500
        const val EGRESS_SOCKET_TIMEOUT_MS = 5_000
        const val VPN_MTU = 1500
        const val VPN_TUN_ADDRESS = "172.19.0.1"
        const val VPN_TUN_PREFIX = 30
        const val VPN_DNS_SERVER = "198.18.0.2"
        const val LOCAL_SOCKS_HOST = "127.0.0.1"
        val EGRESS_PROBE_TARGETS = listOf(
            EgressProbeTarget(
                host = "cloudflare.com",
                port = 80,
                request = "GET /cdn-cgi/trace HTTP/1.1\r\nHost: cloudflare.com\r\nConnection: close\r\n\r\n",
            ),
            EgressProbeTarget(
                host = "ip-api.com",
                port = 80,
                request = "GET /line/?fields=country,countryCode,query HTTP/1.1\r\nHost: ip-api.com\r\nConnection: close\r\n\r\n",
            ),
            EgressProbeTarget(
                host = "connectivitycheck.gstatic.com",
                port = 80,
                request = "GET /generate_204 HTTP/1.1\r\nHost: connectivitycheck.gstatic.com\r\nConnection: close\r\n\r\n",
            ),
            EgressProbeTarget(
                host = "example.com",
                port = 80,
                request = "HEAD / HTTP/1.1\r\nHost: example.com\r\nConnection: close\r\n\r\n",
            ),
            EgressProbeTarget(
                host = "neverssl.com",
                port = 80,
                request = "HEAD / HTTP/1.1\r\nHost: neverssl.com\r\nConnection: close\r\n\r\n",
            ),
        )
    }
}
