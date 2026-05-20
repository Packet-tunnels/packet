package com.resolo.packet

import android.app.Activity
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Intent
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.widget.Button
import android.widget.ImageButton
import android.widget.ProgressBar
import android.widget.ScrollView
import android.widget.TextView
import android.widget.Toast
import org.json.JSONArray
import org.json.JSONObject
import java.io.EOFException
import java.io.InputStream
import java.net.InetSocketAddress
import java.net.Socket
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import javax.crypto.Mac
import javax.crypto.spec.SecretKeySpec
import javax.net.ssl.SNIHostName
import javax.net.ssl.SSLContext
import javax.net.ssl.SSLSocket

class AutoScannerActivity : Activity() {
    private lateinit var backButton: ImageButton
    private lateinit var startScanButton: Button
    private lateinit var scanProgressText: TextView
    private lateinit var scanProgressBar: ProgressBar
    private lateinit var resultLogText: TextView
    private lateinit var resultScrollView: ScrollView
    private lateinit var exportJsonButton: Button

    private val uiHandler = Handler(Looper.getMainLooper())
    private var isScanning = false
    private var scanThread: Thread? = null
    private var baseConfiguration: TunnelConfiguration? = null
    private val scannerLogLock = Any()
    private val scannerLogLines = mutableListOf<String>()

    private val scanLogCallback = object : PacketBridge.LogCallback {
        override fun onLog(message: String) {
            TunnelLogStore.append(applicationContext, message.trimEnd())
        }
    }

    private val scanResultsArray = JSONArray()
    private val probeTargets = listOf(
        ProbeTarget(
            host = "example.com",
            port = 80,
            request = "HEAD / HTTP/1.1\r\nHost: example.com\r\nConnection: close\r\n\r\n",
        ),
        ProbeTarget(
            host = "connectivitycheck.gstatic.com",
            port = 80,
            request = "GET /generate_204 HTTP/1.1\r\nHost: connectivitycheck.gstatic.com\r\nConnection: close\r\n\r\n",
        ),
        ProbeTarget(
            host = "neverssl.com",
            port = 80,
            request = "HEAD / HTTP/1.1\r\nHost: neverssl.com\r\nConnection: close\r\n\r\n",
        ),
        ProbeTarget(
            host = "cloudflare.com",
            port = 80,
            request = "HEAD / HTTP/1.1\r\nHost: cloudflare.com\r\nConnection: close\r\n\r\n",
        ),
        ProbeTarget(
            host = "www.google.com",
            port = 80,
            request = "HEAD / HTTP/1.1\r\nHost: www.google.com\r\nConnection: close\r\n\r\n",
        ),
    )

    private val snisToTest = listOf(
        "", // No SNI (blank)
        "www.test.com",
        "speedtest.net",
        "www.speedtest.net",
        "www.google.com",
        "cloudflare.com",
        "zoom.us",
        "zula.ir",
        "skyroom.online"
    )

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(R.layout.activity_auto_scanner)
        forceLeftToRightLayout()

        backButton = findViewById(R.id.backButton)
        startScanButton = findViewById(R.id.startScanButton)
        scanProgressText = findViewById(R.id.scanProgressText)
        scanProgressBar = findViewById(R.id.scanProgressBar)
        resultLogText = findViewById(R.id.resultLogText)
        resultScrollView = findViewById(R.id.resultScrollView)
        exportJsonButton = findViewById(R.id.exportJsonButton)

        PacketBridge.setLogCallback(scanLogCallback)
        baseConfiguration = TunnelPreferences.loadConfiguration(this)

        backButton.setOnClickListener { finish() }
        
        startScanButton.setOnClickListener {
            if (!isScanning) {
                startScanner()
            } else {
                Toast.makeText(this, "Scan in progress...", Toast.LENGTH_SHORT).show()
            }
        }

        exportJsonButton.setOnClickListener {
            exportResults()
        }
    }

    override fun onDestroy() {
        super.onDestroy()
        scanThread?.interrupt()
        PacketBridge.stopClient()
    }

    private fun startScanner() {
        if (TunnelPreferences.loadSnapshot(this).state.isActive) {
            Toast.makeText(this, "Disconnect the current tunnel before running the scanner.", Toast.LENGTH_LONG).show()
            return
        }

        if (baseConfiguration?.normalizedServerUrl.isNullOrBlank()) {
            Toast.makeText(this, "Please set a Server URL in Main Settings first.", Toast.LENGTH_LONG).show()
            return
        }

        isScanning = true
        startScanButton.isEnabled = false
        startScanButton.alpha = 0.5f
        exportJsonButton.isEnabled = false
        exportJsonButton.alpha = 0.5f

        while (scanResultsArray.length() > 0) {
            scanResultsArray.remove(0)
        }
        synchronized(scannerLogLock) {
            scannerLogLines.clear()
        }

        resultLogText.text = "Initializing Scanner...\n"
        scanProgressText.text = "Preparing profiles..."

        // Build profiles
        val profiles = mutableListOf<TestProfile>()

        // 1. Base config test (as configured by user)
        profiles.add(TestProfile(
            name = "Base User Config (${baseConfiguration!!.transportLabel})",
            cdnEdge = baseConfiguration!!.normalizedCdnEdge,
            hostOverride = baseConfiguration!!.normalizedHostOverride,
            sniOverride = baseConfiguration!!.normalizedSniOverride,
            transportMode = baseConfiguration!!.transportMode,
            fragmentEnabled = baseConfiguration!!.fragmentEnabled,
            fragmentSizeValue = baseConfiguration!!.fragmentSizeValue
        ))

        // 2. HTTP Port 80 plain CDN
        profiles.add(TestProfile(
            name = "HTTP Plain CDN:80",
            cdnEdge = "185.239.1.185:80",
            hostOverride = baseConfiguration!!.normalizedHostOverride.ifBlank { baseConfiguration!!.serverHost },
            sniOverride = "",
            transportMode = TunnelTransportMode.HTTP
        ))
        
        // 3. WS Port 80 plain CDN
        profiles.add(TestProfile(
            name = "WS Plain CDN:80",
            cdnEdge = "185.239.1.185:80",
            hostOverride = baseConfiguration!!.normalizedHostOverride.ifBlank { baseConfiguration!!.serverHost },
            sniOverride = "",
            transportMode = TunnelTransportMode.WEBSOCKET
        ))

        // 4. Primary CDN on 443 with the endpoint host as SNI.
        profiles.add(TestProfile(
            name = "WS + Default SNI",
            cdnEdge = "185.239.1.185:443",
            hostOverride = baseConfiguration!!.normalizedHostOverride.ifBlank { baseConfiguration!!.serverHost },
            sniOverride = "",
            transportMode = TunnelTransportMode.WEBSOCKET
        ))

        profiles.add(TestProfile(
            name = "Stealth HTTPS + Default SNI",
            cdnEdge = "185.239.1.185:443",
            hostOverride = baseConfiguration!!.normalizedHostOverride.ifBlank { baseConfiguration!!.serverHost },
            sniOverride = "",
            transportMode = TunnelTransportMode.STEALTH
        ))

        // 5. Spoofed SNIs over port 443 with WebSocket
        snisToTest.filter { it.isNotBlank() }.forEach { sni ->
            profiles.add(TestProfile(
                name = "WS + SNI: $sni",
                cdnEdge = "185.239.1.185:443", // Try the primary CDN on 443
                hostOverride = baseConfiguration!!.normalizedHostOverride.ifBlank { baseConfiguration!!.serverHost },
                sniOverride = sni,
                transportMode = TunnelTransportMode.WEBSOCKET
            ))
            
            profiles.add(TestProfile(
                name = "WS + SNI: $sni (Fragmented)",
                cdnEdge = "185.239.1.185:443",
                hostOverride = baseConfiguration!!.normalizedHostOverride.ifBlank { baseConfiguration!!.serverHost },
                sniOverride = sni,
                transportMode = TunnelTransportMode.WEBSOCKET,
                fragmentEnabled = true,
                fragmentSizeValue = 40
            ))

            profiles.add(TestProfile(
                name = "Stealth + SNI: $sni",
                cdnEdge = "185.239.1.185:443",
                hostOverride = baseConfiguration!!.normalizedHostOverride.ifBlank { baseConfiguration!!.serverHost },
                sniOverride = sni,
                transportMode = TunnelTransportMode.STEALTH
            ))
        }

        scanProgressBar.max = profiles.size
        scanProgressBar.progress = 0

        scanThread = Thread {
            runScanLoop(profiles)
        }
        scanThread?.start()
    }

    private fun runScanLoop(profiles: List<TestProfile>) {
        appendLog("Starting Auto-Scan with ${profiles.size} profiles...")
        val directNetworkCache = mutableMapOf<String, UserNetworkReachability>()

        if (!sleepInterruptibly(500)) {
            finishScan(successCount = 0, totalProfiles = profiles.size)
            return
        }

        updateProgress("Checking direct network reachability...", 0)
        val baseline = runUserNetworkBaseline()
        scanResultsArray.put(
            JSONObject()
                .put("entryType", "user_network_baseline")
                .put("status", "info")
                .put("summary", baseline.summary)
                .put("reachableCount", baseline.reachableCount)
                .put("checks", baseline.checks.toDirectJsonArray())
        )
        appendLog(".. User network baseline: ${baseline.summary}")

        var successCount = 0

        for ((index, profile) in profiles.withIndex()) {
            if (Thread.currentThread().isInterrupted) break

            updateProgress("Testing ${index + 1}/${profiles.size}: ${profile.name}", index + 1)
            appendLog("\n>> ---- Test ${index + 1}: ${profile.name} ----")
            appendLog(">> CDN: ${profile.cdnEdge} | SNI: ${profile.sniOverride} | Host: ${profile.hostOverride}")

            val resultObj = JSONObject()
            resultObj.put("profileName", profile.name)
            resultObj.put("cdnEdge", profile.cdnEdge)
            resultObj.put("sniOverride", profile.sniOverride)
            resultObj.put("hostOverride", profile.hostOverride)
            resultObj.put("transport", profile.transportMode.title)

            val directNetwork = directNetworkCache.getOrPut(profile.networkCacheKey()) {
                runProfileUserNetworkChecks(profile)
            }
            resultObj.put("userNetworkSummary", directNetwork.summary)
            resultObj.put("userNetworkReachable", directNetwork.anySucceeded)
            resultObj.put("userNetworkChecks", directNetwork.checks.toDirectJsonArray())
            directNetwork.clue?.let { resultObj.put("userNetworkClue", it) }
            appendLog(".. User network: ${directNetwork.summary}")
            directNetwork.clue?.let { appendLog(".. User network clue: $it") }

            val startedAt = System.currentTimeMillis()
            var runtimeSnapshot = TunnelRuntimeSnapshot.empty

            try {
                PacketBridge.stopClient()

                val listenPort = PacketBridge.startClientFull(
                    baseConfiguration!!.normalizedServerUrl,
                    baseConfiguration!!.normalizedSecret,
                    0,
                    profile.cdnEdge,
                    profile.hostOverride,
                    profile.sniOverride,
                    profile.transportMode.rawValue,
                    profile.fragmentEnabled,
                    profile.fragmentSizeValue,
                    profile.obfsKey,
                    baseConfiguration!!.normalizedUpstreamProxy
                )

                resultObj.put("listenPort", listenPort)

                if (listenPort < 0) {
                    appendLog("!! startClientFull returned error code $listenPort")
                    resultObj.put("status", "error_start")
                    resultObj.put("error", "code $listenPort")
                } else {
                    appendLog(".. Local SOCKS5 is listening on 127.0.0.1:$listenPort")

                    val proxyReady = waitForLocalProxy(listenPort)
                    resultObj.put("proxyReady", proxyReady)
                    if (!proxyReady) {
                        resultObj.put("status", "proxy_unreachable")
                        resultObj.put("error", "Local SOCKS5 listener never answered on 127.0.0.1:$listenPort")
                        appendLog("!! Local SOCKS5 listener never answered on 127.0.0.1:$listenPort")
                    } else {
                        val readiness = waitForTransportReadiness(profile.transportMode)
                        runtimeSnapshot = TunnelRuntimeSnapshot.fromJsonString(PacketBridge.copyStatsJson())
                        resultObj.put("transportReady", runtimeSnapshot.tunnelActive)
                        resultObj.put("runtimeState", runtimeSnapshot.state)
                        resultObj.put("runtimeTransport", runtimeSnapshot.transport)
                        resultObj.put("runtimeTrace", readiness.trace)
                        resultObj.put("transportWaitMs", readiness.waitedMs)
                        runtimeSnapshot.lastPingMs?.let { resultObj.put("runtimePingMs", it) }

                        deriveProfileClue(profile, directNetwork, runtimeSnapshot)?.let {
                            resultObj.put("clue", it)
                        }

                        if (!runtimeSnapshot.lastError.isNullOrBlank() && !runtimeSnapshot.tunnelActive) {
                            appendLog("❌ Runtime error: ${runtimeSnapshot.lastError}")
                            resultObj.put("status", "failed")
                            resultObj.put("error", runtimeSnapshot.lastError)
                        } else if (!runtimeSnapshot.tunnelActive) {
                            val skipReason = if (readiness.timedOut) {
                                "Transport never became active before ${readiness.waitedMs}ms deadline (state=${runtimeSnapshot.state})"
                            } else {
                                "Transport not active yet (state=${runtimeSnapshot.state})"
                            }
                            appendLog("❌ $skipReason")
                            resultObj.put("status", "failed")
                            resultObj.put("error", skipReason)
                            resultObj.put("probeSkipped", true)
                            resultObj.put("probeSkipReason", skipReason)
                        } else {
                            val probe = probeTunnelThroughSocks(listenPort)
                            runtimeSnapshot = TunnelRuntimeSnapshot.fromJsonString(PacketBridge.copyStatsJson())

                            resultObj.put("probeTarget", probe.target)
                            resultObj.put("socksConnectOk", probe.connectSucceeded)
                            resultObj.put("httpProbeOk", probe.httpResponseReceived)
                            resultObj.put("successfulTargets", probe.successfulTargetCount)
                            resultObj.put("targetResults", probe.attempts.toJsonArray())
                            if (!probe.responsePreview.isNullOrBlank()) {
                                resultObj.put("responsePreview", probe.responsePreview)
                            }
                            if (!probe.error.isNullOrBlank()) {
                                resultObj.put("probeError", probe.error)
                            }

                            if (probe.connectSucceeded) {
                                successCount++
                                val verificationLabel = if (probe.httpResponseReceived) {
                                    "HTTP response received"
                                } else {
                                    "SOCKS CONNECT confirmed"
                                }
                                appendLog("✅ $verificationLabel via ${probe.target}. Ping ms = ${runtimeSnapshot.lastPingMs ?: "unknown"}")
                                resultObj.put("status", "success")
                                resultObj.put("verification", verificationLabel)
                            } else if (!runtimeSnapshot.lastError.isNullOrBlank()) {
                                appendLog("❌ Runtime error: ${runtimeSnapshot.lastError}")
                                resultObj.put("status", "failed")
                                resultObj.put("error", runtimeSnapshot.lastError)
                            } else {
                                appendLog("❌ Probe failed: ${probe.error ?: "remote CONNECT never succeeded"}")
                                resultObj.put("status", "failed")
                                resultObj.put("error", probe.error ?: "remote CONNECT never succeeded")
                            }
                        }
                    }
                }
            } catch (e: Exception) {
                appendLog("!! Exception: ${e.localizedMessage}")
                resultObj.put("status", "exception")
                resultObj.put("error", e.localizedMessage)
            }

            val elapsedMs = System.currentTimeMillis() - startedAt
            resultObj.put("timeMs", elapsedMs)
            if (runtimeSnapshot.lastPingMs != null) {
                resultObj.put("pingMs", runtimeSnapshot.lastPingMs)
            }
            if (!runtimeSnapshot.lastError.isNullOrBlank() && !resultObj.has("runtimeError")) {
                resultObj.put("runtimeError", runtimeSnapshot.lastError)
            }
            if (!resultObj.has("clue")) {
                deriveProfileClue(profile, directNetwork, runtimeSnapshot)?.let { resultObj.put("clue", it) }
            }
            attachFailureLogs(resultObj)

            scanResultsArray.put(resultObj)
            PacketBridge.stopClient()

            if (!sleepInterruptibly(400)) {
                break
            }
        }

        PacketBridge.stopClient()
        finishScan(successCount, profiles.size)
    }

    private fun finishScan(successCount: Int, totalProfiles: Int) {
        val completedText = "Scan Complete. Working profiles: $successCount / $totalProfiles"
        updateProgress(completedText, totalProfiles)
        appendLog("\n============ DONE ============\n$completedText")

        uiHandler.post {
            isScanning = false
            startScanButton.isEnabled = true
            startScanButton.text = "Run Again"
            startScanButton.alpha = 1.0f
            exportJsonButton.isEnabled = true
            exportJsonButton.alpha = 1.0f
        }
    }

    private fun runUserNetworkBaseline(): UserNetworkBaseline {
        val checks = probeTargets.take(3).map { target ->
            probeDirectRequest(
                label = "baseline-http",
                target = "${target.host}:${target.port}",
                connectHost = target.host,
                connectPort = target.port,
                rawRequest = target.request,
                useTls = false,
                tlsSni = null,
            )
        }

        val reachableCount = checks.count { it.appResponseReceived }
        val summary = when {
            reachableCount == checks.size -> "Direct internet baseline reachable for ${checks.size}/${checks.size} targets"
            reachableCount > 0 -> "Direct internet baseline reachable for $reachableCount/${checks.size} targets"
            else -> "Direct internet baseline failed for all ${checks.size} targets"
        }

        return UserNetworkBaseline(
            checks = checks,
            summary = summary,
            reachableCount = reachableCount,
        )
    }

    private fun runProfileUserNetworkChecks(profile: TestProfile): UserNetworkReachability {
        val configuration = baseConfiguration ?: return UserNetworkReachability(
            checks = emptyList(),
            summary = "No base configuration loaded",
            clue = "Scanner could not load base configuration",
            anySucceeded = false,
        )

        val endpoint = resolveProfileEndpoint(profile, configuration)
        val hostHeader = profile.hostOverride.ifBlank { configuration.serverHost }
        val checks = mutableListOf<DirectNetworkCheck>()

        when (profile.transportMode) {
            TunnelTransportMode.HTTP,
            TunnelTransportMode.STEALTH,
            TunnelTransportMode.MEEK -> {
                checks += runHttpNetworkChecks(
                    endpoint = endpoint,
                    hostHeader = hostHeader,
                    sni = profile.sniOverride.ifBlank { hostHeader },
                    secret = configuration.normalizedSecret,
                    secure = endpoint.port == 443,
                )
            }

            TunnelTransportMode.WEBSOCKET -> {
                checks += runWebSocketNetworkCheck(
                    endpoint = endpoint,
                    hostHeader = hostHeader,
                    useTls = endpoint.port == 443,
                    sni = profile.sniOverride.ifBlank { hostHeader },
                )
            }

            TunnelTransportMode.OBFS -> {
                checks += probeDirectTcp(
                    label = "obfs-tcp",
                    target = "${endpoint.host}:${endpoint.port}",
                    connectHost = endpoint.host,
                    connectPort = endpoint.port,
                )
            }

            TunnelTransportMode.AUTO -> {
                checks += runHttpNetworkChecks(
                    endpoint = endpoint,
                    hostHeader = hostHeader,
                    sni = profile.sniOverride.ifBlank { hostHeader },
                    secret = configuration.normalizedSecret,
                    secure = endpoint.port == 443,
                )
                checks += runWebSocketNetworkCheck(
                    endpoint = endpoint,
                    hostHeader = hostHeader,
                    useTls = endpoint.port == 443,
                    sni = profile.sniOverride.ifBlank { hostHeader },
                )
            }
        }

        return UserNetworkReachability(
            checks = checks,
            summary = buildUserNetworkSummary(checks),
            clue = buildUserNetworkClue(profile, checks),
            anySucceeded = checks.any { it.connectSucceeded || it.appResponseReceived },
        )
    }

    private fun probeDirectTcp(
        label: String,
        target: String,
        connectHost: String,
        connectPort: Int,
    ): DirectNetworkCheck {
        val startedAt = System.currentTimeMillis()
        return try {
            Socket().use { socket ->
                socket.connect(InetSocketAddress(connectHost, connectPort), 4_000)
            }
            DirectNetworkCheck(
                label = label,
                target = target,
                connectSucceeded = true,
                appResponseReceived = false,
                statusLine = null,
                responsePreview = null,
                error = null,
                durationMs = System.currentTimeMillis() - startedAt,
            )
        } catch (e: Exception) {
            DirectNetworkCheck(
                label = label,
                target = target,
                connectSucceeded = false,
                appResponseReceived = false,
                statusLine = null,
                responsePreview = null,
                error = e.localizedMessage ?: e.javaClass.simpleName,
                durationMs = System.currentTimeMillis() - startedAt,
            )
        }
    }

    private fun runHttpNetworkChecks(
        endpoint: EndpointTarget,
        hostHeader: String,
        sni: String,
        secret: String,
        secure: Boolean,
    ): List<DirectNetworkCheck> {
        val requestPrefix = if (secure) "https" else "http"
        val rootRequest = buildHttpRequest(
            method = "GET",
            path = "/",
            hostHeader = hostHeader,
            body = null,
        )
        val authRequest = buildHttpRequest(
            method = "POST",
            path = "/api/v1/auth/login",
            hostHeader = hostHeader,
            body = buildAuthRequestBody(secret),
        )

        return listOf(
            probeDirectRequest(
                label = "$requestPrefix-root",
                target = "${endpoint.host}:${endpoint.port}",
                connectHost = endpoint.host,
                connectPort = endpoint.port,
                rawRequest = rootRequest,
                useTls = secure,
                tlsSni = if (secure) sni else null,
            ),
            probeDirectRequest(
                label = "$requestPrefix-auth",
                target = "${endpoint.host}:${endpoint.port}",
                connectHost = endpoint.host,
                connectPort = endpoint.port,
                rawRequest = authRequest,
                useTls = secure,
                tlsSni = if (secure) sni else null,
            ),
        )
    }

    private fun runWebSocketNetworkCheck(
        endpoint: EndpointTarget,
        hostHeader: String,
        useTls: Boolean,
        sni: String,
    ): DirectNetworkCheck {
        val request = buildWebSocketUpgradeRequest(
            hostHeader = hostHeader,
            originScheme = if (useTls) "https" else "http",
        )

        return probeDirectRequest(
            label = if (useTls) "wss-upgrade" else "ws-upgrade",
            target = "${endpoint.host}:${endpoint.port}",
            connectHost = endpoint.host,
            connectPort = endpoint.port,
            rawRequest = request,
            useTls = useTls,
            tlsSni = if (useTls) sni else null,
        )
    }

    private fun resolveProfileEndpoint(
        profile: TestProfile,
        configuration: TunnelConfiguration,
    ): EndpointTarget {
        val rawEdge = profile.cdnEdge.trim()
        if (rawEdge.isNotEmpty()) {
            val host = rawEdge.substringBefore(":").trim()
            val port = rawEdge.substringAfter(":", "").trim().toIntOrNull()
                ?: if (configuration.normalizedServerUrl.startsWith("https")) 443 else 80
            return EndpointTarget(host = host, port = port)
        }

        return EndpointTarget(
            host = configuration.endpointHost,
            port = configuration.endpointPort,
        )
    }

    private fun buildHttpRequest(
        method: String,
        path: String,
        hostHeader: String,
        body: String?,
    ): String {
        val builder = StringBuilder()
            .append(method)
            .append(' ')
            .append(path)
            .append(" HTTP/1.1\r\n")
            .append("Host: ")
            .append(hostHeader)
            .append("\r\n")
            .append("Connection: close\r\n")
            .append("User-Agent: PacketScanner/1.0\r\n")

        if (body != null) {
            builder
                .append("Content-Type: application/json\r\n")
                .append("Content-Length: ")
                .append(body.toByteArray(Charsets.UTF_8).size)
                .append("\r\n")
        }

        builder.append("\r\n")
        if (body != null) {
            builder.append(body)
        }

        return builder.toString()
    }

    private fun buildWebSocketUpgradeRequest(
        hostHeader: String,
        originScheme: String,
    ): String {
        return buildString {
            append("GET /api/v1/lessons/live HTTP/1.1\r\n")
            append("Host: $hostHeader\r\n")
            append("Upgrade: websocket\r\n")
            append("Connection: Upgrade\r\n")
            append("Sec-WebSocket-Version: 13\r\n")
            append("Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n")
            append("Origin: $originScheme://$hostHeader\r\n")
            append("User-Agent: PacketScanner/1.0\r\n")
            append("\r\n")
        }
    }

    private fun buildAuthRequestBody(secret: String): String {
        val timestamp = System.currentTimeMillis() / 1_000L
        val signature = buildAuthSignature(secret, timestamp)
        return """{"ts":$timestamp,"sig":"$signature"}"""
    }

    private fun buildAuthSignature(secret: String, timestamp: Long): String {
        val mac = Mac.getInstance("HmacSHA256")
        val key = SecretKeySpec(secret.toByteArray(Charsets.UTF_8), "HmacSHA256")
        mac.init(key)
        val digest = mac.doFinal(timestamp.toString().toByteArray(Charsets.UTF_8))
        val builder = StringBuilder(digest.size * 2)
        digest.forEach { byte ->
            builder.append(((byte.toInt() ushr 4) and 0xF).toString(16))
            builder.append((byte.toInt() and 0xF).toString(16))
        }
        return builder.toString()
    }

    private fun probeDirectRequest(
        label: String,
        target: String,
        connectHost: String,
        connectPort: Int,
        rawRequest: String,
        useTls: Boolean,
        tlsSni: String?,
    ): DirectNetworkCheck {
        val startedAt = System.currentTimeMillis()
        var connectSucceeded = false
        var appResponseReceived = false
        var statusLine: String? = null
        var responsePreview: String? = null
        var errorMessage: String? = null

        try {
            openUserNetworkSocket(
                host = connectHost,
                port = connectPort,
                useTls = useTls,
                tlsSni = tlsSni,
            ).use { socket ->
                connectSucceeded = true
                val output = socket.getOutputStream()
                val input = socket.getInputStream()

                output.write(rawRequest.toByteArray(Charsets.US_ASCII))
                output.flush()

                val buffer = ByteArray(768)
                val bytesRead = input.read(buffer)
                if (bytesRead <= 0) {
                    errorMessage = "no response bytes returned"
                } else {
                    appResponseReceived = true
                    val rawResponse = String(buffer, 0, bytesRead, Charsets.US_ASCII)
                    statusLine = rawResponse
                        .lineSequence()
                        .firstOrNull()
                        ?.trim()
                        ?.takeIf { it.startsWith("HTTP/") }
                    responsePreview = rawResponse
                        .replace('\u0000', ' ')
                        .replace('\r', ' ')
                        .replace('\n', ' ')
                        .take(220)
                }
            }
        } catch (e: Exception) {
            errorMessage = e.localizedMessage ?: e.javaClass.simpleName
        }

        return DirectNetworkCheck(
            label = label,
            target = target,
            connectSucceeded = connectSucceeded,
            appResponseReceived = appResponseReceived,
            statusLine = statusLine,
            responsePreview = responsePreview,
            error = errorMessage,
            durationMs = System.currentTimeMillis() - startedAt,
        )
    }

    private fun openUserNetworkSocket(
        host: String,
        port: Int,
        useTls: Boolean,
        tlsSni: String?,
    ): Socket {
        val socket = Socket()
        socket.soTimeout = 8_000
        socket.connect(InetSocketAddress(host, port), 4_000)

        if (!useTls) {
            return socket
        }

        val context = SSLContext.getInstance("TLS")
        context.init(null, null, null)
        val sslSocket = context.socketFactory.createSocket(socket, host, port, true) as SSLSocket
        sslSocket.soTimeout = 8_000

        if (!tlsSni.isNullOrBlank() && Build.VERSION.SDK_INT >= Build.VERSION_CODES.N) {
            val sslParameters = sslSocket.sslParameters
            sslParameters.serverNames = listOf(SNIHostName(tlsSni))
            sslSocket.sslParameters = sslParameters
        }

        sslSocket.startHandshake()
        return sslSocket
    }

    private fun buildUserNetworkSummary(checks: List<DirectNetworkCheck>): String {
        if (checks.isEmpty()) {
            return "No direct network checks were run"
        }

        return checks.joinToString(" | ") { check ->
            val result = when {
                !check.statusLine.isNullOrBlank() -> check.statusLine
                !check.error.isNullOrBlank() -> check.error
                check.connectSucceeded -> "connected"
                else -> "no response"
            }
            "${check.label}=$result"
        }
    }

    private fun buildUserNetworkClue(
        profile: TestProfile,
        checks: List<DirectNetworkCheck>,
    ): String? {
        val httpAuth = checks.firstOrNull { it.label.endsWith("-auth") }
        val wsUpgrade = checks.firstOrNull { it.label.contains("upgrade") && it.label.contains("ws") }

        if (httpAuth?.isHttpSuccess() == true && wsUpgrade?.statusCode() == 400) {
            return "Raw network can reach HTTP auth on this edge, but WebSocket upgrade is rejected. Force HTTP instead of Auto/WS."
        }

        if (httpAuth?.isTimeout() == true && checks.any { it.label.contains("root") && it.isHttpSuccess() }) {
            return "The edge homepage answers, but auth POST times out. CDN forwarding for /api or POST traffic may be blocked on this network."
        }

        if (httpAuth?.isTimeout() == true && wsUpgrade?.isTimeout() == true) {
            return "Both HTTP auth and WebSocket timed out before origin. This looks like edge reachability or filtering on the user's network."
        }

        if (httpAuth?.statusCode() in listOf(403, 404, 421, 502, 503)) {
            return "The CDN/origin answered auth, but rejected the route. Check CDN forwarding rules for ${profile.hostOverride.ifBlank { "the configured host" }}."
        }

        if (wsUpgrade?.statusCode() == 400) {
            return "This edge answers quickly but rejects the WebSocket upgrade. Avoid WS on this host/port."
        }

        if (wsUpgrade?.error?.contains("timed out", ignoreCase = true) == true && httpAuth?.isHttpSuccess() == true) {
            return "HTTP works on this network, but WebSocket is blackholed. Force HTTP transport."
        }

        return null
    }

    private fun deriveProfileClue(
        profile: TestProfile,
        directNetwork: UserNetworkReachability,
        runtimeSnapshot: TunnelRuntimeSnapshot,
    ): String? {
        if (
            profile.transportMode == TunnelTransportMode.AUTO &&
            !runtimeSnapshot.tunnelActive &&
            runtimeSnapshot.lastError?.contains("WebSocket", ignoreCase = true) == true
        ) {
            val httpAuth = directNetwork.checks.firstOrNull { it.label.endsWith("-auth") }
            if (httpAuth?.isHttpSuccess() == true) {
                return "Auto is failing on its WebSocket stage first, but direct HTTP auth works. Force HTTP on this network."
            }
        }

        if (
            profile.transportMode == TunnelTransportMode.HTTP &&
            !runtimeSnapshot.tunnelActive &&
            runtimeSnapshot.lastError?.contains("timed out", ignoreCase = true) == true &&
            directNetwork.checks.any { it.isTimeout() }
        ) {
            return "The tunnel client timed out at the same point as the raw network probe. This looks like edge/IP reachability from the user's network, not an Android app bug."
        }

        return directNetwork.clue
    }

    private fun waitForLocalProxy(listenPort: Int): Boolean {
        repeat(20) {
            if (Thread.currentThread().isInterrupted) {
                return false
            }

            val connected = runCatching {
                Socket().use { socket ->
                    socket.connect(InetSocketAddress("127.0.0.1", listenPort), 400)
                }
            }.isSuccess

            if (connected) {
                return true
            }

            if (!sleepInterruptibly(200)) {
                return false
            }
        }

        return false
    }

    private fun waitForTransportReadiness(transportMode: TunnelTransportMode): TransportReadiness {
        val timeoutMs = when (transportMode) {
            TunnelTransportMode.HTTP -> 40_000L
            TunnelTransportMode.WEBSOCKET -> 18_000L
            TunnelTransportMode.STEALTH -> 40_000L
            TunnelTransportMode.OBFS -> 40_000L
            TunnelTransportMode.MEEK -> 60_000L
            TunnelTransportMode.AUTO -> 25_000L
        }
        val startedAt = System.currentTimeMillis()
        val deadline = startedAt + timeoutMs
        var latestSnapshot = TunnelRuntimeSnapshot.fromJsonString(PacketBridge.copyStatsJson())
        val trace = JSONArray()
        var lastTraceKey: String? = null

        appendLog(".. Waiting up to ${timeoutMs / 1000}s for ${transportMode.title} transport readiness")

        while (System.currentTimeMillis() < deadline) {
            if (Thread.currentThread().isInterrupted) {
                return TransportReadiness(
                    snapshot = latestSnapshot,
                    trace = trace,
                    timedOut = true,
                    waitedMs = System.currentTimeMillis() - startedAt,
                )
            }

            latestSnapshot = TunnelRuntimeSnapshot.fromJsonString(PacketBridge.copyStatsJson())
            val traceEntry = JSONObject()
                .put("state", latestSnapshot.state)
                .put("transport", latestSnapshot.transport)
                .put("tunnelActive", latestSnapshot.tunnelActive)
                .put("lastPingMs", latestSnapshot.lastPingMs)
                .put("lastError", latestSnapshot.lastError)

            val traceKey = traceEntry.toString()
            if (traceKey != lastTraceKey) {
                trace.put(traceEntry)
                appendLog(
                    ".. Runtime state=${latestSnapshot.state} transport=${latestSnapshot.transport} " +
                        "active=${latestSnapshot.tunnelActive} ping=${latestSnapshot.lastPingMs ?: "-"} " +
                        "error=${latestSnapshot.lastError ?: "-"}"
                )
                lastTraceKey = traceKey
            }
            if (latestSnapshot.tunnelActive || shouldStopOnRuntimeError(transportMode, latestSnapshot)) {
                return TransportReadiness(
                    snapshot = latestSnapshot,
                    trace = trace,
                    timedOut = false,
                    waitedMs = System.currentTimeMillis() - startedAt,
                )
            }

            if (!sleepInterruptibly(200)) {
                return TransportReadiness(
                    snapshot = latestSnapshot,
                    trace = trace,
                    timedOut = true,
                    waitedMs = System.currentTimeMillis() - startedAt,
                )
            }
        }

        return TransportReadiness(
            snapshot = latestSnapshot,
            trace = trace,
            timedOut = true,
            waitedMs = System.currentTimeMillis() - startedAt,
        )
    }

    private fun shouldStopOnRuntimeError(
        transportMode: TunnelTransportMode,
        snapshot: TunnelRuntimeSnapshot,
    ): Boolean {
        if (snapshot.lastError.isNullOrBlank()) {
            return false
        }

        if (transportMode != TunnelTransportMode.AUTO) {
            return true
        }

        return snapshot.state !in setOf("starting", "connecting", "reconnecting", "http-fallback")
    }

    private fun probeTunnelThroughSocks(listenPort: Int): SocksProbeResult {
        val failures = mutableListOf<String>()
        val attempts = mutableListOf<TargetProbeResult>()

        for (target in probeTargets) {
            if (Thread.currentThread().isInterrupted) {
                return SocksProbeResult(
                    target = "${target.host}:${target.port}",
                    connectSucceeded = false,
                    httpResponseReceived = false,
                    responsePreview = null,
                    error = "scan interrupted",
                    attempts = attempts,
                    successfulTargetCount = attempts.count { it.httpResponseReceived },
                )
            }

            val targetLabel = "${target.host}:${target.port}"
            val startedAt = System.currentTimeMillis()
            var connectSucceeded = false
            var httpResponseReceived = false
            var responsePreview: String? = null
            var errorMessage: String? = null

            try {
                Socket().use { socket ->
                    socket.soTimeout = 12_000
                    socket.connect(InetSocketAddress("127.0.0.1", listenPort), 1_000)

                    val input = socket.getInputStream()
                    val output = socket.getOutputStream()

                    output.write(byteArrayOf(0x05.toByte(), 0x01.toByte(), 0x00.toByte()))
                    output.flush()

                    val authReply = readExact(input, 2)
                    if (authReply[0].toInt() != 0x05 || authReply[1].toInt() != 0x00) {
                        throw IllegalStateException("SOCKS5 auth failed: ${authReply.joinToString()}")
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
                        throw IllegalStateException("Invalid SOCKS5 reply version: ${connectReply[0].toInt() and 0xFF}")
                    }
                    if (connectReply[1].toInt() != 0x00) {
                        throw IllegalStateException(
                            "SOCKS5 CONNECT failed: ${socksReplyCodeLabel(connectReply[1].toInt() and 0xFF)}",
                        )
                    }
                    consumeSocksBindAddress(input, connectReply[3].toInt() and 0xFF)
                    connectSucceeded = true

                    output.write(target.request.toByteArray(Charsets.US_ASCII))
                    output.flush()

                    val responseBuffer = ByteArray(256)
                    val bytesRead = input.read(responseBuffer)
                    if (bytesRead <= 0) {
                        errorMessage = "CONNECT succeeded but no HTTP bytes returned"
                    } else {
                        httpResponseReceived = true
                        responsePreview = String(responseBuffer, 0, bytesRead, Charsets.US_ASCII)
                            .replace('\n', ' ')
                            .replace('\r', ' ')
                            .take(140)
                    }
                }
            } catch (e: Exception) {
                errorMessage = e.localizedMessage ?: e.javaClass.simpleName
            }

            val durationMs = System.currentTimeMillis() - startedAt
            attempts += TargetProbeResult(
                target = targetLabel,
                connectSucceeded = connectSucceeded,
                httpResponseReceived = httpResponseReceived,
                responsePreview = responsePreview,
                error = errorMessage,
                durationMs = durationMs,
            )

            if (httpResponseReceived) {
                appendLog(".. Probe success $targetLabel in ${durationMs}ms")
                return SocksProbeResult(
                    target = targetLabel,
                    connectSucceeded = true,
                    httpResponseReceived = true,
                    responsePreview = responsePreview,
                    error = null,
                    attempts = attempts,
                    successfulTargetCount = attempts.count { it.httpResponseReceived },
                )
            }

            failures += "$targetLabel -> ${errorMessage ?: "no HTTP response"}"
            appendLog(".. Probe failed $targetLabel in ${durationMs}ms: ${errorMessage ?: "no HTTP response"}")
        }

        return SocksProbeResult(
            target = probeTargets.joinToString(", ") { "${it.host}:${it.port}" },
            connectSucceeded = attempts.any { it.connectSucceeded },
            httpResponseReceived = false,
            responsePreview = attempts.firstNotNullOfOrNull { it.responsePreview },
            error = failures.joinToString(" | "),
            attempts = attempts,
            successfulTargetCount = attempts.count { it.httpResponseReceived },
        )
    }

    private fun readExact(input: InputStream, byteCount: Int): ByteArray {
        val buffer = ByteArray(byteCount)
        var offset = 0
        while (offset < byteCount) {
            val read = input.read(buffer, offset, byteCount - offset)
            if (read < 0) {
                throw EOFException("Expected $byteCount bytes, got $offset")
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
            else -> throw IllegalStateException("Unknown SOCKS5 address type $addressType")
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
            else -> "reply code $code"
        }
    }

    private fun sleepInterruptibly(durationMs: Long): Boolean {
        return try {
            Thread.sleep(durationMs)
            true
        } catch (_: InterruptedException) {
            Thread.currentThread().interrupt()
            false
        }
    }

    private fun appendLog(msg: String) {
        val stamp = SimpleDateFormat("HH:mm:ss", Locale.US).format(Date())
        val formattedLine = "[$stamp] $msg"
        synchronized(scannerLogLock) {
            scannerLogLines.add(formattedLine)
            if (scannerLogLines.size > MAX_SCANNER_LOG_LINES) {
                scannerLogLines.subList(0, scannerLogLines.size - MAX_SCANNER_LOG_LINES).clear()
            }
        }
        TunnelLogStore.append(this, "[SCAN] $msg")
        uiHandler.post {
            val current = resultLogText.text.toString()
            resultLogText.text = "$current\n$formattedLine"
            resultScrollView.post { resultScrollView.fullScroll(ScrollView.FOCUS_DOWN) }
        }
    }

    private fun updateProgress(msg: String, progress: Int) {
        uiHandler.post {
            scanProgressText.text = msg
            scanProgressBar.progress = progress
        }
    }

    private fun exportResults() {
        val clipboard = getSystemService(ClipboardManager::class.java)
        val jsonString = scanResultsArray.toString(2)
        clipboard.setPrimaryClip(ClipData.newPlainText("Packet Scan Results", jsonString))
        Toast.makeText(this, "JSON Report copied to clipboard!", Toast.LENGTH_SHORT).show()
        
        // Optionally launch share intent
        val shareIntent = Intent().apply {
            action = Intent.ACTION_SEND
            putExtra(Intent.EXTRA_TEXT, jsonString)
            type = "text/plain"
        }
        startActivity(Intent.createChooser(shareIntent, "Share JSON Report"))
    }

    private fun attachFailureLogs(resultObj: JSONObject) {
        if (resultObj.optString("status") == "success") {
            return
        }

        resultObj.put("scannerLogTail", recentScannerLogTail())
        resultObj.put("appLogTail", recentAppLogTail())
        resultObj.put("runtimeStatsJson", PacketBridge.copyStatsJson() ?: "{}")
    }

    private fun recentScannerLogTail(limit: Int = 80): JSONArray {
        val lines = synchronized(scannerLogLock) {
            scannerLogLines.takeLast(limit)
        }
        return lines.toJsonStringArray()
    }

    private fun recentAppLogTail(limit: Int = 80): JSONArray {
        return TunnelLogStore.load(this).takeLast(limit).toJsonStringArray()
    }

    data class TestProfile(
        val name: String,
        val cdnEdge: String,
        val hostOverride: String,
        val sniOverride: String,
        val transportMode: TunnelTransportMode,
        val fragmentEnabled: Boolean = false,
        val fragmentSizeValue: Int = 40,
        val obfsKey: String = "",
    ) {
        fun networkCacheKey(): String {
            return listOf(
                transportMode.title,
                cdnEdge,
                hostOverride,
                sniOverride,
                obfsKey,
            ).joinToString("|")
        }
    }

    data class EndpointTarget(
        val host: String,
        val port: Int,
    )

    data class ProbeTarget(
        val host: String,
        val port: Int,
        val request: String,
    )

    data class SocksProbeResult(
        val target: String,
        val connectSucceeded: Boolean,
        val httpResponseReceived: Boolean,
        val responsePreview: String?,
        val error: String?,
        val attempts: List<TargetProbeResult>,
        val successfulTargetCount: Int,
    )

    data class TargetProbeResult(
        val target: String,
        val connectSucceeded: Boolean,
        val httpResponseReceived: Boolean,
        val responsePreview: String?,
        val error: String?,
        val durationMs: Long,
    )

    data class TransportReadiness(
        val snapshot: TunnelRuntimeSnapshot,
        val trace: JSONArray,
        val timedOut: Boolean,
        val waitedMs: Long,
    )

    data class UserNetworkBaseline(
        val checks: List<DirectNetworkCheck>,
        val summary: String,
        val reachableCount: Int,
    )

    data class UserNetworkReachability(
        val checks: List<DirectNetworkCheck>,
        val summary: String,
        val clue: String?,
        val anySucceeded: Boolean,
    )

    data class DirectNetworkCheck(
        val label: String,
        val target: String,
        val connectSucceeded: Boolean,
        val appResponseReceived: Boolean,
        val statusLine: String?,
        val responsePreview: String?,
        val error: String?,
        val durationMs: Long,
    ) {
        fun statusCode(): Int? {
            return statusLine
                ?.split(' ')
                ?.getOrNull(1)
                ?.toIntOrNull()
        }

        fun isHttpSuccess(): Boolean {
            return statusCode() in 200..299
        }

        fun isTimeout(): Boolean {
            return error?.contains("timed out", ignoreCase = true) == true
        }
    }

    private fun List<TargetProbeResult>.toJsonArray(): JSONArray {
        val array = JSONArray()
        forEach { attempt ->
            array.put(
                JSONObject()
                    .put("target", attempt.target)
                    .put("connectSucceeded", attempt.connectSucceeded)
                    .put("httpResponseReceived", attempt.httpResponseReceived)
                    .put("responsePreview", attempt.responsePreview)
                    .put("error", attempt.error)
                    .put("durationMs", attempt.durationMs)
            )
        }
        return array
    }

    private fun List<DirectNetworkCheck>.toDirectJsonArray(): JSONArray {
        val array = JSONArray()
        forEach { check ->
            array.put(
                JSONObject()
                    .put("label", check.label)
                    .put("target", check.target)
                    .put("connectSucceeded", check.connectSucceeded)
                    .put("appResponseReceived", check.appResponseReceived)
                    .put("statusLine", check.statusLine)
                    .put("responsePreview", check.responsePreview)
                    .put("error", check.error)
                    .put("durationMs", check.durationMs)
            )
        }
        return array
    }

    private fun List<String>.toJsonStringArray(): JSONArray {
        val array = JSONArray()
        forEach { line -> array.put(line) }
        return array
    }

    private companion object {
        const val MAX_SCANNER_LOG_LINES = 500
    }
}
