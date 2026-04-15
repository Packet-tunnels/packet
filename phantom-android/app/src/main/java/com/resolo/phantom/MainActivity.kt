package com.resolo.phantom

import android.annotation.SuppressLint
import android.app.Activity
import android.app.Dialog
import android.content.ClipData
import android.content.ClipboardManager
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.res.ColorStateList
import android.graphics.Color
import android.graphics.drawable.ColorDrawable
import android.graphics.drawable.GradientDrawable
import android.net.VpnService
import android.os.Build
import android.os.Bundle
import android.text.InputType
import android.text.Layout
import android.text.format.Formatter
import android.view.Gravity
import android.view.MotionEvent
import android.view.View
import android.view.ViewGroup
import android.view.WindowManager
import android.widget.ArrayAdapter
import android.widget.Button
import android.widget.EditText
import android.widget.ImageButton
import android.widget.ScrollView
import android.widget.Spinner
import android.widget.TextView
import android.widget.Toast
import java.util.Locale
import kotlin.math.max

private data class RuntimeRateAnchor(
    val timestampMs: Long,
    val bytesUp: Long,
    val bytesDown: Long,
)

private data class StatusPalette(
    val accent: Int,
    val accentSoft: Int,
    val bannerBackground: Int,
    val bannerText: Int,
)

class MainActivity : Activity() {
    private lateinit var statusText: TextView
    private lateinit var statusDetailText: TextView
    private lateinit var statusBannerText: TextView
    private lateinit var statusBadge: View
    private lateinit var statusIndicator: View
    private lateinit var statusBadgeText: TextView
    private lateinit var statusTimerText: TextView
    private lateinit var configPrimaryText: TextView
    private lateinit var configSecondaryText: TextView
    private lateinit var configDetailText: TextView
    private lateinit var metricTransportValue: TextView
    private lateinit var metricTransportDetail: TextView
    private lateinit var metricEndpointValue: TextView
    private lateinit var metricEndpointDetail: TextView
    private lateinit var metricPingValue: TextView
    private lateinit var metricPingDetail: TextView
    private lateinit var metricStreamsValue: TextView
    private lateinit var metricStreamsDetail: TextView
    private lateinit var metricDownloadValue: TextView
    private lateinit var metricDownloadDetail: TextView
    private lateinit var metricUploadValue: TextView
    private lateinit var metricUploadDetail: TextView
    private lateinit var settingsButton: ImageButton
    private lateinit var startButton: Button
    private lateinit var logsMetaText: TextView
    private lateinit var toggleLogsButton: Button
    private lateinit var logsContent: View
    private lateinit var logsView: TextView
    private lateinit var logsScrollView: ScrollView
    private lateinit var bottomBar: View

    private var currentConfiguration = TunnelConfiguration()
    private var pendingStartAfterPermission = false
    private var receiverRegistered = false
    private var rateAnchor: RuntimeRateAnchor? = null
    private var logsCollapsed = true

    private val logCallback = object : PhantomTunnel.LogCallback {
        override fun onLog(message: String) {
            runOnUiThread {
                TunnelLogStore.append(applicationContext, message.trimEnd())
                renderLogs()
            }
        }
    }

    private val tunnelEventReceiver = object : BroadcastReceiver() {
        override fun onReceive(context: Context?, intent: Intent?) {
            when (intent?.action) {
                TunnelActions.ACTION_LOG_UPDATED -> renderLogs()
                TunnelActions.ACTION_STATE_UPDATED -> {
                    val state = intent.getStringExtra("state")
                    val message = intent.getStringExtra("message")
                    if (context != null && state != null && message != null) {
                        TunnelPreferences.syncStateLocally(context, state, message)
                    }
                    renderState()
                }
                TunnelActions.ACTION_DASHBOARD_UPDATED -> {
                    val runtime = intent.getStringExtra("runtime")
                    val diagnostics = intent.getStringExtra("diagnostics")
                    if (context != null && (runtime != null || diagnostics != null)) {
                        TunnelPreferences.syncDashboardLocally(context, runtime, diagnostics)
                    }
                    renderDashboard()
                }
            }
        }
    }

    @SuppressLint("SetTextI18n", "ClickableViewAccessibility")
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(R.layout.activity_main)

        statusText = findViewById(R.id.statusText)
        statusDetailText = findViewById(R.id.statusDetailText)
        statusBannerText = findViewById(R.id.statusBannerText)
        statusBadge = findViewById(R.id.statusBadge)
        statusIndicator = findViewById(R.id.statusIndicator)
        statusBadgeText = findViewById(R.id.statusBadgeText)
        statusTimerText = findViewById(R.id.statusTimerText)
        configPrimaryText = findViewById(R.id.configPrimaryText)
        configSecondaryText = findViewById(R.id.configSecondaryText)
        configDetailText = findViewById(R.id.configDetailText)
        metricTransportValue = findViewById(R.id.metricTransportValue)
        metricTransportDetail = findViewById(R.id.metricTransportDetail)
        metricEndpointValue = findViewById(R.id.metricEndpointValue)
        metricEndpointDetail = findViewById(R.id.metricEndpointDetail)
        metricPingValue = findViewById(R.id.metricPingValue)
        metricPingDetail = findViewById(R.id.metricPingDetail)
        metricStreamsValue = findViewById(R.id.metricStreamsValue)
        metricStreamsDetail = findViewById(R.id.metricStreamsDetail)
        metricDownloadValue = findViewById(R.id.metricDownloadValue)
        metricDownloadDetail = findViewById(R.id.metricDownloadDetail)
        metricUploadValue = findViewById(R.id.metricUploadValue)
        metricUploadDetail = findViewById(R.id.metricUploadDetail)
        settingsButton = findViewById(R.id.settingsButton)
        startButton = findViewById(R.id.startButton)
        logsMetaText = findViewById(R.id.logsMetaText)
        toggleLogsButton = findViewById(R.id.toggleLogsButton)
        logsContent = findViewById(R.id.logsContent)
        logsView = findViewById(R.id.logsView)
        logsScrollView = findViewById(R.id.logsScrollView)
        bottomBar = findViewById(R.id.bottomBar)

        currentConfiguration = TunnelPreferences.loadConfiguration(this)
        PhantomTunnel.setLogCallback(logCallback)
        applyBottomBarInsets()
        renderConfigurationSummary()
        renderState()
        renderDashboard()
        renderLogs()

        if (TunnelLogStore.load(this).isEmpty()) {
            TunnelLogStore.append(this, "[APP] Android VPN controller is ready")
            TunnelLogStore.append(this, "[APP] Rust JNI bridge loaded")
            renderLogs()
        }

        settingsButton.setOnClickListener {
            showSettingsDialog()
        }

        toggleLogsButton.setOnClickListener {
            setLogsCollapsed(!logsCollapsed)
        }

        startButton.setOnTouchListener { view, event ->
            when (event.action) {
                MotionEvent.ACTION_DOWN -> view.alpha = 0.7f
                MotionEvent.ACTION_UP, MotionEvent.ACTION_CANCEL -> view.alpha = if (view.isEnabled) 1.0f else 0.5f
            }
            false
        }

        startButton.setOnClickListener {
            toggleTunnel()
        }

        findViewById<Button>(R.id.clearLogsButton).setOnClickListener {
            TunnelLogStore.clear(this)
            renderLogs()
        }

        findViewById<Button>(R.id.copyLogsButton).setOnClickListener {
            copyLogsToClipboard()
        }

        findViewById<Button>(R.id.testOutputButton).setOnClickListener {
            PhantomTunnel.emitTestOutput()
            TunnelLogStore.append(this, "[APP] Requested Rust test output")
        }

        logsView.setHorizontallyScrolling(false)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.M) {
            logsView.breakStrategy = Layout.BREAK_STRATEGY_HIGH_QUALITY
            logsView.hyphenationFrequency = Layout.HYPHENATION_FREQUENCY_NONE
        }
        setLogsCollapsed(logsCollapsed)
    }

    override fun onStart() {
        super.onStart()
        registerTunnelReceiver()
        currentConfiguration = TunnelPreferences.loadConfiguration(this)
        reconcileVpnPermissionState()
        renderConfigurationSummary()
        renderState()
        renderDashboard()
        renderLogs()
    }

    override fun onResume() {
        super.onResume()
        reconcileVpnPermissionState()
        renderState()
        renderDashboard()
    }

    override fun onStop() {
        if (receiverRegistered) {
            unregisterReceiver(tunnelEventReceiver)
            receiverRegistered = false
        }
        super.onStop()
    }

    @Deprecated("Deprecated in Java")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)

        if (requestCode != REQUEST_VPN_PERMISSION) {
            return
        }

        if (resultCode == RESULT_OK && pendingStartAfterPermission) {
            TunnelLogStore.append(this, "[APP] Android VPN permission approved")
            startTunnelService(TunnelActions.ACTION_CONNECT)
        } else {
            TunnelPreferences.updateState(this, TunnelState.IDLE, "VPN permission was cancelled")
            TunnelLogStore.append(this, "[APP] Android VPN permission was cancelled")
        }

        pendingStartAfterPermission = false
        renderState()
        renderDashboard()
    }

    private fun toggleTunnel() {
        when (TunnelPreferences.loadSnapshot(this).state) {
            TunnelState.RUNNING,
            TunnelState.CONNECTING,
            TunnelState.DISCONNECTING -> requestDisconnect()
            TunnelState.REQUESTING_PERMISSION,
            TunnelState.IDLE,
            TunnelState.FAILED -> requestConnect()
        }
    }

    private fun requestConnect() {
        TunnelLogStore.append(this, "[APP] Connect requested")
        val validationError = validate(currentConfiguration)
        if (validationError != null) {
            TunnelPreferences.updateState(this, TunnelState.FAILED, validationError)
            TunnelLogStore.append(this, "[APP] $validationError")
            renderState()
            return
        }

        TunnelPreferences.saveConfiguration(this, currentConfiguration)
        renderConfigurationSummary()

        val prepareIntent = VpnService.prepare(this)
        if (prepareIntent != null) {
            pendingStartAfterPermission = true
            TunnelPreferences.updateState(
                this,
                TunnelState.REQUESTING_PERMISSION,
                "Approve Android VPN permission in the system dialog",
            )
            TunnelLogStore.append(this, "[APP] Opening Android VPN approval dialog")
            runCatching {
                startActivityForResult(prepareIntent, REQUEST_VPN_PERMISSION)
            }.onFailure { error ->
                pendingStartAfterPermission = false
                TunnelPreferences.updateState(
                    this,
                    TunnelState.FAILED,
                    "Failed to open Android VPN approval",
                )
                TunnelLogStore.append(
                    this,
                    "[APP] Failed to open Android VPN approval: ${error.localizedMessage ?: error.javaClass.simpleName}",
                )
            }
        } else {
            TunnelLogStore.append(this, "[APP] Android VPN permission already granted")
            startTunnelService(TunnelActions.ACTION_CONNECT)
        }
    }

    private fun requestDisconnect() {
        TunnelPreferences.updateState(this, TunnelState.DISCONNECTING, "Stopping Android VPN service")
        startTunnelService(TunnelActions.ACTION_DISCONNECT)
    }

    private fun startTunnelService(action: String) {
        when (action) {
            TunnelActions.ACTION_CONNECT ->
                TunnelLogStore.append(this, "[APP] Start requested through Android VpnService")
            TunnelActions.ACTION_DISCONNECT ->
                TunnelLogStore.append(this, "[APP] Stop requested through Android VpnService")
        }
        val intent = Intent(this, TunnelVpnService::class.java).setAction(action)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            startForegroundService(intent)
        } else {
            startService(intent)
        }
    }

    private fun registerTunnelReceiver() {
        if (receiverRegistered) {
            return
        }

        val filter = IntentFilter().apply {
            addAction(TunnelActions.ACTION_LOG_UPDATED)
            addAction(TunnelActions.ACTION_STATE_UPDATED)
            addAction(TunnelActions.ACTION_DASHBOARD_UPDATED)
        }

        if (Build.VERSION.SDK_INT >= 33) {
            registerReceiver(tunnelEventReceiver, filter, RECEIVER_NOT_EXPORTED)
        } else {
            registerReceiver(tunnelEventReceiver, filter)
        }

        receiverRegistered = true
    }

    private fun renderState() {
        val snapshot = TunnelPreferences.loadSnapshot(this)
        renderStatusPanel(
            snapshot = snapshot,
            runtime = TunnelPreferences.loadRuntimeSnapshot(this),
            diagnostics = TunnelPreferences.loadDiagnostics(this),
        )
        startButton.text = when (snapshot.state) {
            TunnelState.REQUESTING_PERMISSION -> "Approve VPN"
            TunnelState.CONNECTING -> "Starting..."
            TunnelState.DISCONNECTING -> "Stopping..."
            TunnelState.RUNNING -> "Disconnect Tunnel"
            else -> "Connect Tunnel"
        }
        startButton.isEnabled = snapshot.state != TunnelState.CONNECTING &&
            snapshot.state != TunnelState.DISCONNECTING

        startButton.alpha = if (startButton.isEnabled) 1.0f else 0.5f

        val colorHex = when (snapshot.state) {
            TunnelState.RUNNING -> "#EF4444"
            TunnelState.CONNECTING -> "#F59E0B"
            TunnelState.DISCONNECTING -> "#6B7280"
            else -> "#2563EB"
        }
        startButton.backgroundTintList = ColorStateList.valueOf(Color.parseColor(colorHex))
    }

    private fun reconcileVpnPermissionState() {
        val snapshot = TunnelPreferences.loadSnapshot(this)
        if (snapshot.state != TunnelState.REQUESTING_PERMISSION) {
            return
        }

        if (VpnService.prepare(this) == null) {
            if (pendingStartAfterPermission) {
                pendingStartAfterPermission = false
                TunnelLogStore.append(this, "[APP] Android VPN approval returned; starting tunnel service")
                startTunnelService(TunnelActions.ACTION_CONNECT)
                return
            }

            TunnelPreferences.updateState(this, TunnelState.IDLE, "VPN permission is ready")
            TunnelLogStore.append(this, "[APP] Android VPN permission is already granted")
            return
        }

        if (!pendingStartAfterPermission) {
            TunnelPreferences.updateState(
                this,
                TunnelState.IDLE,
                "Tap Connect to open the Android VPN approval dialog",
            )
            TunnelLogStore.append(
                this,
                "[APP] Android VPN approval is still required; tap Connect to open the system dialog again",
            )
        }
    }

    private fun renderConfigurationSummary() {
        val server = currentConfiguration.normalizedServerUrl.ifEmpty { "No server configured" }
        val port = currentConfiguration.listenPortValue?.toString()
            ?: currentConfiguration.listenPort.takeIf { it.isNotBlank() && !it.equals("auto", ignoreCase = true) }
            ?: "Auto"

        configPrimaryText.text = server

        val secondaryParts = mutableListOf("Port $port", currentConfiguration.transportLabel)
        secondaryParts += currentConfiguration.ingressLabel
        configSecondaryText.text = secondaryParts.joinToString(" · ")

        val detailLines = mutableListOf<String>()
        if (currentConfiguration.normalizedServerUrl.isBlank()) {
            detailLines += "Server host: Not set"
            detailLines += "Endpoint: Not set"
        } else {
            detailLines += "Server host: ${currentConfiguration.serverHost}"
            detailLines += "Endpoint: ${currentConfiguration.endpointHost}:${currentConfiguration.endpointPort}"
        }
        if (currentConfiguration.normalizedHostOverride.isNotEmpty()) {
            detailLines += "Host override: ${currentConfiguration.normalizedHostOverride}"
        }
        if (currentConfiguration.normalizedCdnEdge.isNotEmpty()) {
            detailLines += "CDN edge: ${currentConfiguration.normalizedCdnEdge}"
        }
        if (currentConfiguration.normalizedSniOverride.isNotEmpty()) {
            detailLines += "SNI override: ${currentConfiguration.normalizedSniOverride}"
        }
        configDetailText.text = detailLines.joinToString(separator = "\n")
    }

    private fun renderDashboard() {
        val snapshot = TunnelPreferences.loadSnapshot(this)
        val runtime = TunnelPreferences.loadRuntimeSnapshot(this)
        val diagnostics = TunnelPreferences.loadDiagnostics(this)
        val (uploadRateBps, downloadRateBps) = computeRates(runtime)

        renderStatusPanel(snapshot, runtime, diagnostics)

        metricTransportValue.text = runtime.transport.ifBlank { currentConfiguration.transportLabel }
        metricTransportDetail.text = buildList {
            add(runtime.state.ifBlank { snapshot.state.title })
            runtime.listenPort?.let { add("Port $it") }
        }.joinToString(separator = " · ")

        val endpointValue = diagnostics.endpointHost.ifBlank {
            runtime.endpointHost.ifBlank { currentConfiguration.endpointHost }
        }
        metricEndpointValue.text = endpointValue
        metricEndpointDetail.text = when (diagnostics.endpointReachable) {
            true -> diagnostics.endpointLatencyMs?.let { "Reachable in ${it} ms" } ?: "Reachable"
            false -> diagnostics.lastFailureDetail ?: "Endpoint probe failed"
            null -> diagnostics.healthStatus
        }

        val pingValue = runtime.lastPingMs ?: diagnostics.endpointLatencyMs
        metricPingValue.text = pingValue?.let { "$it ms" } ?: "--"
        metricPingDetail.text = when {
            runtime.lastPingMs != null -> "Transport round-trip"
            diagnostics.endpointLatencyMs != null -> "TCP probe"
            else -> "Not measured"
        }

        metricStreamsValue.text = runtime.activeStreams.toString()
        metricStreamsDetail.text = "Total ${runtime.totalStreams}"

        metricDownloadValue.text = formatBytes(runtime.bytesDown)
        metricDownloadDetail.text = formatRate(downloadRateBps)

        metricUploadValue.text = formatBytes(runtime.bytesUp)
        metricUploadDetail.text = formatRate(uploadRateBps)

    }

    private fun renderLogs() {
        val logs = TunnelLogStore.load(this)
        logsMetaText.text = when {
            logs.isEmpty() -> "No logs yet"
            logsCollapsed -> "${logs.size} lines hidden"
            else -> "${logs.size} lines"
        }
        logsView.text = if (logs.isEmpty()) {
            "No logs yet."
        } else {
            logs.joinToString(separator = "\n") { formatLogLineForDisplay(it) }
        }

        if (!logsCollapsed) {
            logsScrollView.post {
                logsScrollView.scrollTo(0, logsView.bottom)
            }
        }
    }

    private fun setLogsCollapsed(collapsed: Boolean) {
        logsCollapsed = collapsed
        logsContent.visibility = if (collapsed) View.GONE else View.VISIBLE
        toggleLogsButton.text = if (collapsed) "Show" else "Hide"
        renderLogs()
    }

    private fun renderStatusPanel(
        snapshot: TunnelSnapshot,
        runtime: TunnelRuntimeSnapshot,
        diagnostics: TunnelDiagnosticsSnapshot,
    ) {
        val palette = statusPalette(snapshot)
        val isRunning = snapshot.state == TunnelState.RUNNING || runtime.tunnelActive

        statusBadge.background = GradientDrawable().apply {
            shape = GradientDrawable.RECTANGLE
            cornerRadius = dpToPx(999f)
            setColor(palette.accentSoft)
        }
        statusIndicator.background = GradientDrawable().apply {
            shape = GradientDrawable.OVAL
            setColor(palette.accent)
        }
        statusBannerText.background = GradientDrawable().apply {
            shape = GradientDrawable.RECTANGLE
            cornerRadius = dpToPx(14f)
            setColor(palette.bannerBackground)
        }

        statusBadgeText.text = when (snapshot.state) {
            TunnelState.RUNNING -> "ACTIVE"
            TunnelState.CONNECTING -> "STARTING"
            TunnelState.REQUESTING_PERMISSION -> "APPROVAL"
            TunnelState.DISCONNECTING -> "STOPPING"
            TunnelState.FAILED -> "ERROR"
            TunnelState.IDLE -> "IDLE"
        }
        statusBadgeText.setTextColor(palette.accent)
        statusTimerText.text = when {
            isRunning && runtime.connectedSince != null -> formatConnectedDuration(runtime.connectedSince)
            snapshot.state == TunnelState.REQUESTING_PERMISSION -> "Awaiting VPN approval"
            snapshot.state == TunnelState.CONNECTING -> "Bringing tunnel online"
            snapshot.state == TunnelState.DISCONNECTING -> "Shutting down"
            snapshot.state == TunnelState.FAILED -> "Needs attention"
            else -> "Not connected"
        }
        statusText.text = when (snapshot.state) {
            TunnelState.RUNNING -> "Tunnel is active"
            TunnelState.CONNECTING -> "Starting tunnel"
            TunnelState.REQUESTING_PERMISSION -> "VPN approval required"
            TunnelState.DISCONNECTING -> "Stopping tunnel"
            TunnelState.FAILED -> "Tunnel failed"
            TunnelState.IDLE -> "Tunnel is idle"
        }
        statusDetailText.text = snapshot.message
        statusBannerText.text = buildStatusBanner(snapshot, runtime, diagnostics)
        statusBannerText.setTextColor(palette.bannerText)
    }

    private fun buildStatusBanner(
        snapshot: TunnelSnapshot,
        runtime: TunnelRuntimeSnapshot,
        diagnostics: TunnelDiagnosticsSnapshot,
    ): String {
        runtime.lastError?.takeIf { it.isNotBlank() }?.let { return it }

        if (snapshot.state == TunnelState.RUNNING && diagnostics.localProxyReady && diagnostics.vpnShellReady) {
            val port = runtime.listenPort?.toString()
                ?: currentConfiguration.listenPortValue?.toString()
                ?: "auto"
            return if (runtime.activeStreams > 0) {
                "Forwarding is active on 127.0.0.1:$port with ${runtime.activeStreams} live stream(s)."
            } else {
                "Forwarding is active on 127.0.0.1:$port. Waiting for device traffic."
            }
        }

        if (diagnostics.recommendation.isNotBlank()) {
            return diagnostics.recommendation
        }

        return snapshot.message
    }

    private fun statusPalette(snapshot: TunnelSnapshot): StatusPalette {
        return when (snapshot.state) {
            TunnelState.RUNNING -> StatusPalette(
                accent = Color.parseColor("#15803D"),
                accentSoft = Color.parseColor("#DCFCE7"),
                bannerBackground = Color.parseColor("#F0FDF4"),
                bannerText = Color.parseColor("#166534"),
            )
            TunnelState.CONNECTING,
            TunnelState.REQUESTING_PERMISSION -> StatusPalette(
                accent = Color.parseColor("#B45309"),
                accentSoft = Color.parseColor("#FEF3C7"),
                bannerBackground = Color.parseColor("#FFFBEB"),
                bannerText = Color.parseColor("#92400E"),
            )
            TunnelState.FAILED -> StatusPalette(
                accent = Color.parseColor("#B91C1C"),
                accentSoft = Color.parseColor("#FEE2E2"),
                bannerBackground = Color.parseColor("#FEF2F2"),
                bannerText = Color.parseColor("#991B1B"),
            )
            TunnelState.DISCONNECTING,
            TunnelState.IDLE -> StatusPalette(
                accent = Color.parseColor("#4B5563"),
                accentSoft = Color.parseColor("#E5E7EB"),
                bannerBackground = Color.parseColor("#F3F4F6"),
                bannerText = Color.parseColor("#374151"),
            )
        }
    }

    private fun formatConnectedDuration(connectedSinceSeconds: Long): String {
        val elapsedSeconds = max(System.currentTimeMillis() / 1000 - connectedSinceSeconds, 0)
        val hours = elapsedSeconds / 3600
        val minutes = (elapsedSeconds % 3600) / 60
        val seconds = elapsedSeconds % 60
        return if (hours > 0) {
            String.format(Locale.US, "%02d:%02d:%02d live", hours, minutes, seconds)
        } else {
            String.format(Locale.US, "%02d:%02d live", minutes, seconds)
        }
    }

    private fun dpToPx(valueDp: Float): Float {
        return valueDp * resources.displayMetrics.density
    }

    private fun computeRates(snapshot: TunnelRuntimeSnapshot): Pair<Double, Double> {
        if (!snapshot.tunnelActive) {
            rateAnchor = null
            return 0.0 to 0.0
        }

        val now = System.currentTimeMillis()
        val anchor = rateAnchor
        var uploadRate = 0.0
        var downloadRate = 0.0

        if (anchor != null &&
            snapshot.bytesUp >= anchor.bytesUp &&
            snapshot.bytesDown >= anchor.bytesDown
        ) {
            val elapsedSeconds = max((now - anchor.timestampMs) / 1000.0, 0.5)
            uploadRate = (snapshot.bytesUp - anchor.bytesUp) / elapsedSeconds
            downloadRate = (snapshot.bytesDown - anchor.bytesDown) / elapsedSeconds
        }

        rateAnchor = RuntimeRateAnchor(
            timestampMs = now,
            bytesUp = snapshot.bytesUp,
            bytesDown = snapshot.bytesDown,
        )
        return uploadRate to downloadRate
    }

    private fun formatBytes(bytes: Long): String {
        return Formatter.formatShortFileSize(this, bytes.coerceAtLeast(0))
    }

    private fun formatRate(bytesPerSecond: Double): String {
        return "${formatBytes(bytesPerSecond.toLong())}/s"
    }

    private fun copyLogsToClipboard() {
        val logs = TunnelLogStore.load(this)
        if (logs.isEmpty()) {
            Toast.makeText(this, "No logs to copy", Toast.LENGTH_SHORT).show()
            return
        }

        val clipboard = getSystemService(ClipboardManager::class.java)
        clipboard.setPrimaryClip(ClipData.newPlainText("Phantom Tunnel Logs", logs.joinToString("\n")))
        Toast.makeText(this, "Logs copied", Toast.LENGTH_SHORT).show()
    }

    private fun formatLogLineForDisplay(line: String): String {
        return line
            .replace("/", "/\u200B")
            .replace(":", ":\u200B")
            .replace(".", ".\u200B")
            .replace("=", "=\u200B")
            .replace("?", "?\u200B")
            .replace("&", "&\u200B")
    }

    private fun applyBottomBarInsets() {
        val baseBottomPadding = bottomBar.paddingBottom
        bottomBar.setOnApplyWindowInsetsListener { view, insets ->
            view.setPadding(
                view.paddingLeft,
                view.paddingTop,
                view.paddingRight,
                baseBottomPadding + insets.systemWindowInsetBottom,
            )
            insets
        }
        bottomBar.requestApplyInsets()
    }

    private fun showSettingsDialog() {
        val dialog = Dialog(this)
        dialog.setContentView(R.layout.dialog_settings)
        dialog.window?.setBackgroundDrawable(ColorDrawable(Color.TRANSPARENT))
        dialog.window?.setWindowAnimations(R.style.PhantomSlideDialogAnimation)
        dialog.setCanceledOnTouchOutside(true)

        val serverUrlInput = dialog.findViewById<EditText>(R.id.settingsServerUrlInput)
        val secretInput = dialog.findViewById<EditText>(R.id.settingsSecretInput)
        val listenPortInput = dialog.findViewById<EditText>(R.id.settingsListenPortInput)
        val cdnEdgeInput = dialog.findViewById<EditText>(R.id.settingsCdnEdgeInput)
        val hostOverrideInput = dialog.findViewById<EditText>(R.id.settingsHostOverrideInput)
        val sniOverrideInput = dialog.findViewById<EditText>(R.id.settingsSniOverrideInput)
        val transportSpinner = dialog.findViewById<Spinner>(R.id.settingsTransportSpinner)
        val secretToggle = dialog.findViewById<ImageButton>(R.id.secretToggleVisibility)
        val errorText = dialog.findViewById<TextView>(R.id.settingsErrorText)

        transportSpinner.adapter = ArrayAdapter(
            this,
            android.R.layout.simple_spinner_item,
            TunnelTransportMode.values().map { it.title },
        ).apply {
            setDropDownViewResource(android.R.layout.simple_spinner_dropdown_item)
        }

        serverUrlInput.setText(currentConfiguration.serverUrl)
        secretInput.setText(currentConfiguration.secret)
        listenPortInput.setText(currentConfiguration.listenPort)
        cdnEdgeInput.setText(currentConfiguration.cdnEdge)
        hostOverrideInput.setText(currentConfiguration.hostOverride)
        sniOverrideInput.setText(currentConfiguration.sniOverride)
        transportSpinner.setSelection(currentConfiguration.transportMode.ordinal)

        var isSecretVisible = false
        secretToggle.setOnClickListener {
            isSecretVisible = !isSecretVisible
            secretInput.inputType = if (isSecretVisible) {
                InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_VISIBLE_PASSWORD
            } else {
                InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_PASSWORD
            }
            secretToggle.setImageResource(
                if (isSecretVisible) {
                    R.drawable.ic_visibility_off
                } else {
                    R.drawable.ic_visibility
                },
            )
            secretInput.setSelection(secretInput.text.length)
        }

        dialog.findViewById<ImageButton>(R.id.settingsCloseButton).setOnClickListener {
            dialog.dismiss()
        }

        dialog.findViewById<Button>(R.id.settingsCancelButton).setOnClickListener {
            dialog.dismiss()
        }

        dialog.findViewById<Button>(R.id.settingsSaveButton).setOnClickListener {
            val updatedConfiguration = TunnelConfiguration(
                serverUrl = serverUrlInput.text.toString(),
                secret = secretInput.text.toString(),
                listenPort = listenPortInput.text.toString(),
                cdnEdge = cdnEdgeInput.text.toString(),
                hostOverride = hostOverrideInput.text.toString(),
                sniOverride = sniOverrideInput.text.toString(),
                transportMode = TunnelTransportMode.values()[transportSpinner.selectedItemPosition],
            )

            val validationError = validate(updatedConfiguration)
            if (validationError != null) {
                errorText.text = validationError
                errorText.visibility = View.VISIBLE
                return@setOnClickListener
            }

            currentConfiguration = updatedConfiguration
            TunnelPreferences.saveConfiguration(this, updatedConfiguration)
            TunnelLogStore.append(this, "[APP] Tunnel settings saved")
            renderConfigurationSummary()
            renderDashboard()
            renderLogs()
            dialog.dismiss()
        }

        dialog.show()
        dialog.window?.apply {
            setGravity(Gravity.BOTTOM)
            setLayout(
                ViewGroup.LayoutParams.MATCH_PARENT,
                WindowManager.LayoutParams.WRAP_CONTENT,
            )
        }
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

    private companion object {
        const val REQUEST_VPN_PERMISSION = 1001
    }
}
