package com.resolo.packet

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.pm.ServiceInfo
import android.content.Intent
import android.net.ConnectivityManager
import android.net.NetworkCapabilities
import android.net.VpnService
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.os.ParcelFileDescriptor
import android.provider.Settings
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
    private var connectInFlight = false
    private var activeConfiguration: TunnelConfiguration? = null
    private var lastRuntimeErrorLogged: String? = null

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

                mainHandler.post {
                    completeTunnelConnection(configuration, listenPort)
                }
            } catch (e: Exception) {
                mainHandler.post {
                    failStart(e.localizedMessage ?: "Failed to start tunnel.")
                }
            }
        }.start()
    }

    private fun completeTunnelConnection(configuration: TunnelConfiguration, listenPort: Int) {
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
        connectInFlight = false
        TunnelPreferences.updateState(this, TunnelState.DISCONNECTING, "Stopping Android VPN service")
        TunnelLogStore.append(this, "[VPN] Stop requested")
        stopTelemetryRefresh()
        closeInterface()
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
        mainHandler.postDelayed({
            android.os.Process.killProcess(android.os.Process.myPid())
        }, 250)
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
        val runtimeSnapshot = TunnelRuntimeSnapshot.fromJsonString(PacketBridge.copyStatsJson())
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

    private fun startRustCore(configuration: TunnelConfiguration): Int {
        val listenPort = configuration.listenPortValue ?: 0
        return if (configuration.usesCdn) {
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

    private fun performPreflightDiagnostics(configuration: TunnelConfiguration): TunnelDiagnosticsSnapshot {
        val endpointProbe = runOnWorkerThread { probeEndpoint(configuration.endpointHost, configuration.endpointPort) }
        val healthProbe = runOnWorkerThread { probeHealth(configuration.normalizedServerUrl) }

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
        TunnelLogStore.append(
            this,
            "[DIAG] Config summary: server_url=${configuration.normalizedServerUrl} server_host=${configuration.serverHost} " +
                "transport=${configuration.transportLabel} listen_port=${configuration.listenPort.ifBlank { "auto" }} " +
                "uses_cdn=${configuration.usesCdn} cdn_edge=${configuration.normalizedCdnEdge.ifBlank { "(empty)" }} " +
                "host_override=${configuration.normalizedHostOverride.ifBlank { "(empty)" }} " +
                "sni_override=${configuration.normalizedSniOverride.ifBlank { "(empty)" }} " +
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
                val routeHint = if (configuration.usesCdn) {
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
                "Full-device forwarding is active. Device traffic is routed through tun2socks into $LOCAL_SOCKS_HOST:${forwardingPort ?: "auto"}."
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
            transport = configuration.transportLabel,
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

    private data class EndpointProbe(
        val reachable: Boolean?,
        val latencyMs: Int?,
        val error: String?,
    )

    private data class HealthProbe(
        val status: String,
        val error: String?,
    )

    private companion object {
        const val NOTIFICATION_CHANNEL_ID = "packet_status"
        const val NOTIFICATION_ID = 1001
        const val TELEMETRY_REFRESH_MS = 1_000L
        const val VPN_MTU = 1500
        const val VPN_TUN_ADDRESS = "172.19.0.1"
        const val VPN_TUN_PREFIX = 30
        const val VPN_DNS_SERVER = "198.18.0.2"
        const val LOCAL_SOCKS_HOST = "127.0.0.1"
    }
}
