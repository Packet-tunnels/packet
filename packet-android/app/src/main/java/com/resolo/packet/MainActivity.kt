package com.resolo.packet

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
import android.os.Handler
import android.os.Looper
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
    private lateinit var disclosureReminderCard: View
    private lateinit var statusBadgeText: TextView
    private lateinit var statusTimerText: TextView
    private lateinit var configStatusSummary: TextView
    private lateinit var statusBannerText: TextView
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
    private lateinit var autoScannerButton: Button
    private lateinit var logsView: TextView
    private lateinit var logsScrollView: ScrollView
    private lateinit var bottomBar: View

    private var currentConfiguration = TunnelConfiguration()
    private var currentConfigurationEntry: SavedTunnelConfiguration? = null
    private var activeConfigurationId: String? = null
    private var activeConfigurationSnapshot: TunnelConfiguration? = null
    private var pendingStartAfterPermission = false
    private var hasPresentedInitialDisclosure = false
    private var receiverRegistered = false
    private var rateAnchor: RuntimeRateAnchor? = null
    private var logsCollapsed = true
    private val uiHandler = Handler(Looper.getMainLooper())
    private val logRenderRunnable = Runnable { renderLogs() }

    private val stalledConnectRunnable = Runnable {
        val stateRecovered = reconcileStalledTunnelState()
        renderState()
        renderDashboard()
        if (stateRecovered) {
            renderLogs()
        }
    }

    private val logCallback = object : PacketBridge.LogCallback {
        override fun onLog(message: String) {
            TunnelLogStore.append(applicationContext, message.trimEnd())
        }
    }

    private val tunnelEventReceiver = object : BroadcastReceiver() {
        override fun onReceive(context: Context?, intent: Intent?) {
            when (intent?.action) {
                TunnelActions.ACTION_LOG_UPDATED -> requestLogRender()
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

        disclosureReminderCard = findViewById(R.id.disclosureReminderCard)
        statusBadgeText = findViewById(R.id.statusBadgeText)
        statusTimerText = findViewById(R.id.statusTimerText)
        configStatusSummary = findViewById(R.id.configStatusSummary)
        statusBannerText = findViewById(R.id.statusBannerText)
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
        autoScannerButton = findViewById(R.id.autoScannerButton)
        toggleLogsButton = findViewById(R.id.toggleLogsButton)
        logsContent = findViewById(R.id.logsContent)
        logsView = findViewById(R.id.logsView)
        logsScrollView = findViewById(R.id.logsScrollView)

        refreshConfigurationState()
        PacketBridge.setLogCallback(logCallback)
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
            startActivity(Intent(this, SettingsActivity::class.java))
        }

        disclosureReminderCard.setOnClickListener {
            showDisclosureDialog(isConnectFlow = false)
        }

        toggleLogsButton.setOnClickListener {
            setLogsCollapsed(!logsCollapsed)
        }

        autoScannerButton.setOnClickListener {
            startActivity(Intent(this, AutoScannerActivity::class.java))
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
            PacketBridge.emitTestOutput()
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
        refreshConfigurationState()
        reconcileStalledTunnelState()
        reconcileVpnPermissionState()
        renderConfigurationSummary()
        renderState()
        renderDashboard()
        renderLogs()
        presentInitialDisclosureIfNeeded()
    }

    override fun onResume() {
        super.onResume()
        refreshConfigurationState()
        reconcileStalledTunnelState()
        reconcileVpnPermissionState()
        renderState()
        renderDashboard()
    }

    override fun onStop() {
        uiHandler.removeCallbacks(stalledConnectRunnable)
        uiHandler.removeCallbacks(logRenderRunnable)
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

        if (resultCode == RESULT_OK) {
            if (pendingStartAfterPermission ||
                TunnelPreferences.loadSnapshot(this).state == TunnelState.REQUESTING_PERMISSION
            ) {
                pendingStartAfterPermission = false
                TunnelLogStore.append(this, "[APP] Android VPN permission approved")
                startTunnelService(TunnelActions.ACTION_CONNECT)
            }
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
        if (!TunnelPreferences.isVpnDisclosureAcknowledged(this)) {
            showDisclosureDialog(
                isConnectFlow = true,
                onAccept = { requestConnectInternal() },
            )
            return
        }

        requestConnectInternal()
    }

    private fun requestConnectInternal() {
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
            TunnelActions.ACTION_CONNECT -> {
                TunnelPreferences.updateState(this, TunnelState.CONNECTING, "Starting Android VPN service")
                TunnelLogStore.append(this, "[APP] Start requested through Android VpnService")
            }
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
        refreshConfigurationState()
        reconcileStalledTunnelState()
        val snapshot = TunnelPreferences.loadSnapshot(this)
        disclosureReminderCard.visibility = if (TunnelPreferences.isVpnDisclosureAcknowledged(this)) {
            View.GONE
        } else {
            View.VISIBLE
        }
        renderStatusPanel(
            snapshot = snapshot,
            runtime = TunnelPreferences.loadRuntimeSnapshot(this),
        )
        startButton.text = when (snapshot.state) {
            TunnelState.REQUESTING_PERMISSION -> "Approve"
            TunnelState.CONNECTING -> "Starting"
            TunnelState.DISCONNECTING -> "Stopping"
            TunnelState.RUNNING -> "Stop"
            else -> "Connect"
        }
        startButton.isEnabled = snapshot.state != TunnelState.CONNECTING &&
            snapshot.state != TunnelState.DISCONNECTING

        startButton.alpha = if (startButton.isEnabled) 1.0f else 0.5f

        val colorHex = when (snapshot.state) {
            TunnelState.RUNNING -> "#EF4444"
            TunnelState.CONNECTING -> "#F59E0B"
            TunnelState.DISCONNECTING -> "#6B7280"
            else -> "#000000"
        }
        startButton.backgroundTintList = ColorStateList.valueOf(Color.parseColor(colorHex))
        scheduleStateWatchdog(snapshot)
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

    private fun reconcileStalledTunnelState(): Boolean {
        val snapshot = TunnelPreferences.loadSnapshot(this)
        if (snapshot.state != TunnelState.CONNECTING) {
            return false
        }

        val stateAgeMs = System.currentTimeMillis() - snapshot.updatedAtMs
        if (stateAgeMs < CONNECT_TIMEOUT_MS) {
            return false
        }

        val timeoutMessage = "Tunnel start timed out. Try again."
        TunnelPreferences.updateRuntimeSnapshot(
            this,
            TunnelPreferences.loadRuntimeSnapshot(this).copy(
                state = "failed",
                connectedSince = null,
                tunnelActive = false,
                lastError = timeoutMessage,
            ),
        )
        TunnelPreferences.updateState(this, TunnelState.FAILED, timeoutMessage)
        TunnelLogStore.append(this, "[APP] $timeoutMessage")
        return true
    }

    private fun scheduleStateWatchdog(snapshot: TunnelSnapshot) {
        uiHandler.removeCallbacks(stalledConnectRunnable)
        if (snapshot.state != TunnelState.CONNECTING || snapshot.updatedAtMs <= 0L) {
            return
        }

        val remainingMs = (CONNECT_TIMEOUT_MS - (System.currentTimeMillis() - snapshot.updatedAtMs))
            .coerceAtLeast(250L)
        uiHandler.postDelayed(stalledConnectRunnable, remainingMs)
    }

    private fun renderConfigurationSummary() {
        refreshConfigurationState()
        val snapshot = TunnelPreferences.loadSnapshot(this)
        val runtime = TunnelPreferences.loadRuntimeSnapshot(this)
        val configuration = displayConfiguration(snapshot, runtime)
        val server = configuration.normalizedServerUrl.ifEmpty { "No server configured" }
        val port = configuration.listenPortValue?.toString()
            ?: configuration.listenPort.takeIf { it.isNotBlank() && !it.equals("auto", ignoreCase = true) }
            ?: "Auto"

        configPrimaryText.text = server

        val secondaryParts = mutableListOf("Port $port", configuration.transportLabel)
        secondaryParts += configuration.ingressLabel
        configSecondaryText.text = secondaryParts.joinToString(" · ")

        val detailLines = mutableListOf<String>()
        if (configuration.normalizedServerUrl.isBlank()) {
            detailLines += "Server host: Not set"
            detailLines += "Endpoint: Not set"
        } else {
            detailLines += "Server host: ${configuration.serverHost}"
            detailLines += "Endpoint: ${configuration.endpointHost}:${configuration.endpointPort}"
        }
        if (configuration.normalizedHostOverride.isNotEmpty()) {
            detailLines += "Host override: ${configuration.normalizedHostOverride}"
        }
        if (configuration.normalizedCdnEdge.isNotEmpty()) {
            detailLines += "CDN edge: ${configuration.normalizedCdnEdge}"
        }
        if (configuration.normalizedSniOverride.isNotEmpty()) {
            detailLines += "SNI override: ${configuration.normalizedSniOverride}"
        }
        if (selectedConfigurationDisplayName().isNotBlank()) {
            detailLines += "Selected profile: ${selectedConfigurationDisplayName()}"
        }
        activeConfigurationDisplayName()?.let { activeName ->
            if (snapshot.state.isActive || runtime.tunnelActive) {
                detailLines += "Active profile: $activeName"
            }
        }
        configDetailText.text = detailLines.joinToString(separator = "\n")
    }

    private fun renderDashboard() {
        refreshConfigurationState()
        val snapshot = TunnelPreferences.loadSnapshot(this)
        val runtime = TunnelPreferences.loadRuntimeSnapshot(this)
        val diagnostics = TunnelPreferences.loadDiagnostics(this)
        val (uploadRateBps, downloadRateBps) = computeRates(runtime)
        val configuration = displayConfiguration(snapshot, runtime)

        renderStatusPanel(snapshot, runtime)
        renderConfigurationSummary()

        metricTransportValue.text = runtime.transport.ifBlank { configuration.transportLabel }
        metricTransportDetail.text = buildList {
            add(runtime.state.ifBlank { snapshot.state.title })
            add("Port ${runtime.listenPort ?: configuration.listenPortValue ?: "Auto"}")
        }.joinToString(separator = " · ")

        val endpointValue = diagnostics.endpointHost.ifBlank {
            runtime.endpointHost.ifBlank { configuration.endpointHost }
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
        if (logs.isEmpty()) {
            logsMetaText.text = "No logs yet"
            logsView.text = "No logs yet."
            return
        }

        if (logsCollapsed) {
            logsMetaText.text = "${logs.size} lines hidden"
            logsView.text = ""
            return
        }

        val visibleLogs = logs.takeLast(MAX_VISIBLE_LOG_LINES)
        logsMetaText.text = if (visibleLogs.size == logs.size) {
            "${logs.size} lines"
        } else {
            "Showing latest ${visibleLogs.size} of ${logs.size} lines"
        }
        logsView.text = visibleLogs.joinToString(separator = "\n") { formatLogLineForDisplay(it) }

        logsScrollView.post {
            logsScrollView.scrollTo(0, logsView.bottom)
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
    ) {
        refreshConfigurationState()
        val palette = statusPalette(snapshot)
        val isRunning = snapshot.state == TunnelState.RUNNING || runtime.tunnelActive
        val configuration = displayConfiguration(snapshot, runtime)
        
        statusBadgeText.text = when (snapshot.state) {
            TunnelState.RUNNING -> "CONNECTED"
            TunnelState.CONNECTING -> "CONNECTING"
            TunnelState.REQUESTING_PERMISSION -> "APPROVAL"
            TunnelState.DISCONNECTING -> "STOPPING"
            TunnelState.FAILED -> "FAILED"
            TunnelState.IDLE -> "DISCONNECTED"
        }
        statusBadgeText.setTextColor(palette.accent)

        if (isRunning && runtime.connectedSince != null) {
            statusTimerText.text = formatConnectedDuration(runtime.connectedSince)
        } else {
            statusTimerText.text = "Not Protected"
        }

        val error = snapshot.message.takeIf { snapshot.state == TunnelState.FAILED }
            ?: runtime.lastError?.takeIf { it.isNotBlank() }
            ?: configuration.validationError
        
        if (error != null) {
            statusBannerText.text = error
            statusBannerText.visibility = View.VISIBLE
        } else {
            statusBannerText.visibility = View.GONE
        }

        configStatusSummary.text = configurationStatusSummary(snapshot, runtime)
    }

    private fun refreshConfigurationState() {
        currentConfigurationEntry = TunnelPreferences.loadSelectedConfigurationEntry(this)
        currentConfiguration = currentConfigurationEntry?.configuration ?: TunnelPreferences.loadConfiguration(this)
        activeConfigurationId = TunnelPreferences.loadActiveConfigurationId(this)
        activeConfigurationSnapshot = TunnelPreferences.loadActiveConfiguration(this)
    }

    private fun displayConfiguration(
        snapshot: TunnelSnapshot,
        runtime: TunnelRuntimeSnapshot,
    ): TunnelConfiguration {
        return if (shouldShowActiveConfiguration(snapshot, runtime) && activeConfigurationSnapshot != null) {
            activeConfigurationSnapshot ?: currentConfiguration
        } else {
            currentConfiguration
        }
    }

    private fun selectedConfigurationDisplayName(): String {
        return currentConfigurationEntry?.displayName
            ?: if (currentConfiguration.isEmpty) "No Configuration" else currentConfiguration.suggestedName
    }

    private fun activeConfigurationDisplayName(): String? {
        return TunnelPreferences.loadActiveConfigurationDisplayName(this)
    }

    private fun configurationStatusSummary(
        snapshot: TunnelSnapshot,
        runtime: TunnelRuntimeSnapshot,
    ): String {
        val selectedName = selectedConfigurationDisplayName()
        val activeName = activeConfigurationDisplayName()

        if (!shouldShowActiveConfiguration(snapshot, runtime) || activeName.isNullOrBlank()) {
            return "Selected configuration: $selectedName"
        }

        if (activeName == selectedName) {
            return "Active configuration: $activeName"
        }

        return "Active now: $activeName. Selected next: $selectedName"
    }

    private fun shouldShowActiveConfiguration(
        snapshot: TunnelSnapshot,
        runtime: TunnelRuntimeSnapshot,
    ): Boolean {
        return runtime.tunnelActive ||
            snapshot.state == TunnelState.RUNNING ||
            snapshot.state == TunnelState.CONNECTING ||
            snapshot.state == TunnelState.DISCONNECTING
    }

    private fun showDisclosureDialog(
        isConnectFlow: Boolean,
        onAccept: (() -> Unit)? = null,
    ) {
        VpnDisclosureDialogs.show(
            activity = this,
            acceptTitle = if (isConnectFlow) "Accept & Connect" else "Acknowledge",
            dismissTitle = if (isConnectFlow) "Not Now" else "Dismiss",
            onAccept = {
                onAccept?.invoke()
                renderState()
            },
        )
    }

    private fun presentInitialDisclosureIfNeeded() {
        if (hasPresentedInitialDisclosure || TunnelPreferences.isVpnDisclosureAcknowledged(this)) {
            return
        }

        hasPresentedInitialDisclosure = true
        showDisclosureDialog(isConnectFlow = false)
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
            String.format(Locale.US, "%02d:%02d:%02d", hours, minutes, seconds)
        } else {
            String.format(Locale.US, "%02d:%02d", minutes, seconds)
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
        val report = TunnelDiagnosticReport.build(this)
        clipboard.setPrimaryClip(ClipData.newPlainText("Packet Diagnostic Report", report))
        Toast.makeText(this, "Diagnostic report copied", Toast.LENGTH_SHORT).show()
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

    private fun requestLogRender() {
        uiHandler.removeCallbacks(logRenderRunnable)
        uiHandler.postDelayed(logRenderRunnable, LOG_RENDER_DEBOUNCE_MS)
    }

    private fun applyBottomBarInsets() {
        // Obsolete with inline button
    }

    private fun validate(configuration: TunnelConfiguration): String? {
        return configuration.validationError
    }

    private companion object {
        const val REQUEST_VPN_PERMISSION = 1001
        const val CONNECT_TIMEOUT_MS = 20_000L
        const val LOG_RENDER_DEBOUNCE_MS = 150L
        const val MAX_VISIBLE_LOG_LINES = 250
    }
}
