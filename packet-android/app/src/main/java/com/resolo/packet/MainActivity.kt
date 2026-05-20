package com.resolo.packet

import android.annotation.SuppressLint
import android.app.Activity
import android.content.ClipData
import android.content.ClipboardManager
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.res.ColorStateList
import android.graphics.Color
import android.graphics.drawable.GradientDrawable
import android.net.VpnService
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.text.Layout
import android.text.format.Formatter
import android.view.LayoutInflater
import android.view.MotionEvent
import android.view.View
import android.view.ViewGroup
import android.view.animation.AccelerateDecelerateInterpolator
import android.widget.Button
import android.widget.EditText
import android.widget.ImageButton
import android.widget.ImageView
import android.widget.LinearLayout
import android.widget.PopupMenu
import android.widget.ScrollView
import android.widget.Switch
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
    private enum class RootTab {
        STATUS,
        SERVERS,
        SETTINGS,
        EDITOR,
        TEST_OUTPUT,
    }

    private lateinit var statusPage: View
    private lateinit var serversPage: View
    private lateinit var settingsPage: View
    private lateinit var configEditorPage: View
    private lateinit var testOutputPage: View
    private lateinit var bottomNavShell: View
    private lateinit var navHome: LinearLayout
    private lateinit var navServers: LinearLayout
    private lateinit var navSettings: LinearLayout
    private lateinit var navHomeIcon: ImageView
    private lateinit var navServersIcon: ImageView
    private lateinit var navSettingsIcon: ImageView
    private lateinit var navHomeLabel: TextView
    private lateinit var navServersLabel: TextView
    private lateinit var navSettingsLabel: TextView
    private lateinit var disclosureReminderCard: View
    private lateinit var statusBadgeText: TextView
    private lateinit var statusTimerText: TextView
    private lateinit var configStatusSummary: TextView
    private lateinit var statusBannerText: TextView
    private lateinit var copyStatusDetailsButton: Button
    private lateinit var configPrimaryText: TextView
    private lateinit var configSecondaryText: TextView
    private lateinit var configDetailText: TextView
    private lateinit var metricTransportValue: TextView
    private lateinit var metricTransportDetail: TextView
    private lateinit var metricEndpointValue: TextView
    private lateinit var metricEndpointDetail: TextView
    private lateinit var metricPingValue: TextView
    private lateinit var metricPingDetail: TextView
    private lateinit var metricCountryValue: TextView
    private lateinit var metricCountryDetail: TextView
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
    private lateinit var diagnosticButton: Button
    private lateinit var logsView: TextView
    private lateinit var logsScrollView: ScrollView
    private lateinit var serversListContainer: LinearLayout
    private lateinit var serversEmptyState: View
    private lateinit var serversAddButton: ImageButton
    private lateinit var serversEditButton: Button
    private lateinit var settingsProfilesSummaryText: TextView
    private lateinit var settingsProfilesRow: View
    private lateinit var settingsProtocolSummaryText: TextView
    private lateinit var settingsTransportSummaryText: TextView
    private lateinit var settingsFragmentationSummaryText: TextView
    private lateinit var settingsPrivacyLinkRow: View
    private lateinit var settingsTermsLinkRow: View
    private lateinit var settingsSupportLinkRow: View
    private lateinit var settingsAboutVersionText: TextView
    private lateinit var headerTestButton: Button
    private lateinit var editorCloseButton: ImageButton
    private lateinit var editorTitleText: TextView
    private lateinit var editorErrorText: TextView
    private lateinit var editorNameInput: EditText
    private lateinit var editorStackValue: TextView
    private lateinit var editorStackModeRow: View
    private lateinit var editorPacketNativeSection: View
    private lateinit var editorDirectSockSection: View
    private lateinit var editorServerUrlInput: EditText
    private lateinit var editorSecretInput: EditText
    private lateinit var editorListenPortInput: EditText
    private lateinit var editorTransportRow: TextView
    private lateinit var editorCdnEdgeInput: EditText
    private lateinit var editorHostOverrideInput: EditText
    private lateinit var editorSniOverrideInput: EditText
    private lateinit var editorObfsKeyInput: EditText
    private lateinit var editorUpstreamProxyInput: EditText
    private lateinit var editorTrojanUriInput: EditText
    private lateinit var editorCarrierPortInput: EditText
    private lateinit var editorFragmentSwitch: Switch
    private lateinit var editorFragmentSizeInput: EditText
    private lateinit var editorSaveButton: Button
    private lateinit var testOutputCloseButton: ImageButton
    private lateinit var testOutputRunButton: Button
    private lateinit var testOutputCopyButton: Button
    private lateinit var testOutputText: TextView
    private lateinit var testOutputScrollView: ScrollView

    private var serversEditMode = false
    private var currentConfiguration = TunnelConfiguration()

    /**
     * One-shot per process: when the user lands on the app and the selected
     * profile is the built-in Packet Chain, kick off Connect automatically
     * so they don't have to tap. Resets to false in `onCreate` so a fresh
     * launch always tries once.
     */
    private var autoConnectAttempted = false
    private var currentConfigurationEntry: SavedTunnelConfiguration? = null
    private var activeConfigurationId: String? = null
    private var activeConfigurationSnapshot: TunnelConfiguration? = null
    private var pendingStartAfterPermission = false
    private var hasPresentedInitialDisclosure = false
    private var receiverRegistered = false
    private var rateAnchor: RuntimeRateAnchor? = null
    private var logsCollapsed = true
    private var selectedRootTab = RootTab.STATUS
    private var testOutputReturnTab = RootTab.STATUS
    private var editorExisting: SavedTunnelConfiguration? = null
    private var editorStackMode = TunnelStackMode.PACKET_NATIVE
    private var editorTransportMode = TunnelTransportMode.AUTO
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
                    val updatedAtMs = intent.getLongExtra("updatedAtMs", System.currentTimeMillis())
                    if (context != null && state != null && message != null) {
                        TunnelPreferences.syncStateLocally(context, state, message, updatedAtMs)
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
        forceLeftToRightLayout()
        configureSystemBars()

        statusPage = findViewById(R.id.rootScrollView)
        serversPage = findViewById(R.id.serversPage)
        settingsPage = findViewById(R.id.settingsPage)
        configEditorPage = findViewById(R.id.configEditorPage)
        testOutputPage = findViewById(R.id.testOutputPage)
        bottomNavShell = findViewById(R.id.bottomNavShell)
        navHome = findViewById(R.id.navHome)
        navServers = findViewById(R.id.navServers)
        navSettings = findViewById(R.id.navSettings)
        navHomeIcon = findViewById(R.id.navHomeIcon)
        navServersIcon = findViewById(R.id.navServersIcon)
        navSettingsIcon = findViewById(R.id.navSettingsIcon)
        navHomeLabel = findViewById(R.id.navHomeLabel)
        navServersLabel = findViewById(R.id.navServersLabel)
        navSettingsLabel = findViewById(R.id.navSettingsLabel)
        disclosureReminderCard = findViewById(R.id.disclosureReminderCard)
        statusBadgeText = findViewById(R.id.statusBadgeText)
        statusTimerText = findViewById(R.id.statusTimerText)
        configStatusSummary = findViewById(R.id.configStatusSummary)
        statusBannerText = findViewById(R.id.statusBannerText)
        copyStatusDetailsButton = findViewById(R.id.copyStatusDetailsButton)
        copyStatusDetailsButton.setOnClickListener { copyStatusDetails() }
        configPrimaryText = findViewById(R.id.configPrimaryText)
        configSecondaryText = findViewById(R.id.configSecondaryText)
        configDetailText = findViewById(R.id.configDetailText)
        metricTransportValue = findViewById(R.id.metricTransportValue)
        metricTransportDetail = findViewById(R.id.metricTransportDetail)
        metricEndpointValue = findViewById(R.id.metricEndpointValue)
        metricEndpointDetail = findViewById(R.id.metricEndpointDetail)
        metricPingValue = findViewById(R.id.metricPingValue)
        metricPingDetail = findViewById(R.id.metricPingDetail)
        metricCountryValue = findViewById(R.id.metricCountryValue)
        metricCountryDetail = findViewById(R.id.metricCountryDetail)
        metricDownloadValue = findViewById(R.id.metricDownloadValue)
        metricDownloadDetail = findViewById(R.id.metricDownloadDetail)
        metricUploadValue = findViewById(R.id.metricUploadValue)
        metricUploadDetail = findViewById(R.id.metricUploadDetail)
        settingsButton = findViewById(R.id.settingsButton)
        startButton = findViewById(R.id.startButton)
        logsMetaText = findViewById(R.id.logsMetaText)
        autoScannerButton = findViewById(R.id.autoScannerButton)
        diagnosticButton = findViewById(R.id.diagnosticButton)
        toggleLogsButton = findViewById(R.id.toggleLogsButton)
        logsContent = findViewById(R.id.logsContent)
        logsView = findViewById(R.id.logsView)
        logsScrollView = findViewById(R.id.logsScrollView)
        serversListContainer = findViewById(R.id.serversListContainer)
        serversEmptyState = findViewById(R.id.serversEmptyState)
        serversAddButton = findViewById(R.id.serversAddButton)
        serversEditButton = findViewById(R.id.serversEditButton)
        settingsProfilesSummaryText = findViewById(R.id.settingsProfilesSummaryText)
        settingsProfilesRow = findViewById(R.id.settingsProfilesRow)
        settingsProtocolSummaryText = findViewById(R.id.settingsProtocolSummaryText)
        settingsTransportSummaryText = findViewById(R.id.settingsTransportSummaryText)
        settingsFragmentationSummaryText = findViewById(R.id.settingsFragmentationSummaryText)
        settingsPrivacyLinkRow = findViewById(R.id.settingsPrivacyLinkRow)
        settingsTermsLinkRow = findViewById(R.id.settingsTermsLinkRow)
        settingsSupportLinkRow = findViewById(R.id.settingsSupportLinkRow)
        settingsAboutVersionText = findViewById(R.id.settingsAboutVersionText)
        headerTestButton = findViewById(R.id.headerTestButton)
        editorCloseButton = findViewById(R.id.editorCloseButton)
        editorTitleText = findViewById(R.id.editorTitleText)
        editorErrorText = findViewById(R.id.editorErrorText)
        editorNameInput = findViewById(R.id.editorNameInput)
        editorStackValue = findViewById(R.id.editorStackValue)
        editorStackModeRow = findViewById(R.id.editorStackModeRow)
        editorPacketNativeSection = findViewById(R.id.editorPacketNativeSection)
        editorDirectSockSection = findViewById(R.id.editorDirectSockSection)
        editorServerUrlInput = findViewById(R.id.editorServerUrlInput)
        editorSecretInput = findViewById(R.id.editorSecretInput)
        editorListenPortInput = findViewById(R.id.editorListenPortInput)
        editorTransportRow = findViewById(R.id.editorTransportRow)
        editorCdnEdgeInput = findViewById(R.id.editorCdnEdgeInput)
        editorHostOverrideInput = findViewById(R.id.editorHostOverrideInput)
        editorSniOverrideInput = findViewById(R.id.editorSniOverrideInput)
        editorObfsKeyInput = findViewById(R.id.editorObfsKeyInput)
        editorUpstreamProxyInput = findViewById(R.id.editorUpstreamProxyInput)
        editorTrojanUriInput = findViewById(R.id.editorTrojanUriInput)
        editorCarrierPortInput = findViewById(R.id.editorCarrierPortInput)
        editorFragmentSwitch = findViewById(R.id.editorFragmentSwitch)
        editorFragmentSizeInput = findViewById(R.id.editorFragmentSizeInput)
        editorSaveButton = findViewById(R.id.editorSaveButton)
        testOutputCloseButton = findViewById(R.id.testOutputCloseButton)
        testOutputRunButton = findViewById(R.id.testOutputRunButton)
        testOutputCopyButton = findViewById(R.id.testOutputCopyButton)
        testOutputText = findViewById(R.id.testOutputText)
        testOutputScrollView = findViewById(R.id.testOutputScrollView)

        refreshConfigurationState()
        PacketBridge.setLogCallback(logCallback)
        applyBottomBarInsets()
        renderRootTab()
        renderConfigurationSummary()
        renderServersPage()
        renderSettingsPage()
        renderState()
        renderDashboard()
        renderLogs()

        if (TunnelLogStore.load(this).isEmpty()) {
            TunnelLogStore.append(this, "[APP] Android VPN controller is ready")
            TunnelLogStore.append(this, "[APP] Rust JNI bridge loaded")
            renderLogs()
        }

        settingsButton.setOnClickListener { selectRootTab(RootTab.SETTINGS) }
        navHome.setOnClickListener { selectRootTab(RootTab.STATUS) }
        navServers.setOnClickListener { selectRootTab(RootTab.SERVERS) }
        navSettings.setOnClickListener { selectRootTab(RootTab.SETTINGS) }
        serversAddButton.setOnClickListener { showConfigurationEditor(existing = null) }
        serversEditButton.setOnClickListener { toggleServersEditMode() }
        settingsPrivacyLinkRow.setOnClickListener { openUrl(PRIVACY_URL) }
        settingsTermsLinkRow.setOnClickListener { openUrl(TERMS_URL) }
        settingsSupportLinkRow.setOnClickListener { openUrl(SUPPORT_URL) }
        settingsProfilesRow.setOnClickListener { selectRootTab(RootTab.SERVERS) }
        headerTestButton.setOnClickListener { openTestOutputPage(runImmediately = true) }
        editorCloseButton.setOnClickListener { selectRootTab(RootTab.SERVERS) }
        editorStackModeRow.setOnClickListener { showStackModeMenu(it) }
        editorTransportRow.setOnClickListener {
            val modes = TunnelTransportMode.values()
            val currentIndex = modes.indexOf(editorTransportMode).coerceAtLeast(0)
            editorTransportMode = modes[(currentIndex + 1) % modes.size]
            renderEditorMode()
        }
        editorSaveButton.setOnClickListener { saveConfigurationEditor() }
        testOutputCloseButton.setOnClickListener { selectRootTab(testOutputReturnTab) }
        testOutputRunButton.setOnClickListener { runTestOutput() }
        testOutputCopyButton.setOnClickListener { copyTestOutputToClipboard() }

        disclosureReminderCard.setOnClickListener {
            showDisclosureDialog(isConnectFlow = false)
        }

        toggleLogsButton.setOnClickListener {
            setLogsCollapsed(!logsCollapsed)
        }

        autoScannerButton.setOnClickListener {
            startActivity(Intent(this, AutoScannerActivity::class.java))
        }

        diagnosticButton.setOnClickListener {
            startActivity(Intent(this, DiagnosticActivity::class.java))
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
            openTestOutputPage(runImmediately = true)
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
        renderServersPage()
        renderSettingsPage()
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
        renderServersPage()
        renderSettingsPage()
        renderState()
        renderDashboard()
        maybeAutoConnectOnLaunch()
    }

    /**
     * If the selected profile is the built-in Packet Chain and the tunnel
     * is currently idle (cold launch, or a previous run failed), trigger
     * the Connect action automatically. Fires at most once per process so
     * the user can still cancel and stay disconnected.
     */
    private fun maybeAutoConnectOnLaunch() {
        if (autoConnectAttempted) return
        if (!currentConfiguration.usesPacketChain) return
        val state = TunnelPreferences.loadSnapshot(this).state
        if (state != TunnelState.IDLE && state != TunnelState.FAILED) return
        autoConnectAttempted = true
        TunnelLogStore.append(
            this,
            "[AUTO-CONNECT] Packet Chain selected — starting tunnel automatically.",
        )
        requestConnect()
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
    @Suppress("DEPRECATION")
    override fun onBackPressed() {
        when (selectedRootTab) {
            RootTab.EDITOR -> selectRootTab(RootTab.SERVERS)
            RootTab.TEST_OUTPUT -> selectRootTab(testOutputReturnTab)
            else -> super.onBackPressed()
        }
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

    private fun selectRootTab(tab: RootTab) {
        if (selectedRootTab == tab) {
            return
        }
        val previousTab = selectedRootTab
        if (selectedRootTab == RootTab.SERVERS && serversEditMode) {
            serversEditMode = false
        }
        selectedRootTab = tab
        renderRootTab(previousTab)
        when (tab) {
            RootTab.STATUS -> {
                renderState()
                renderDashboard()
            }
            RootTab.SERVERS -> renderServersPage()
            RootTab.SETTINGS -> renderSettingsPage()
            RootTab.EDITOR -> renderEditorMode()
            RootTab.TEST_OUTPUT -> renderTestOutput()
        }
    }

    private fun renderRootTab(previousTab: RootTab? = null) {
        if (previousTab == null || previousTab == selectedRootTab) {
            rootPages().forEach { (tab, page) ->
                page.animate().cancel()
                page.translationX = 0f
                page.alpha = 1f
                page.visibility = if (tab == selectedRootTab) View.VISIBLE else View.GONE
            }
        } else {
            animateRootPageTransition(previousTab, selectedRootTab)
        }

        bottomNavShell.visibility =
            if (selectedRootTab == RootTab.EDITOR || selectedRootTab == RootTab.TEST_OUTPUT) View.GONE else View.VISIBLE

        renderNavItem(navHome, navHomeIcon, navHomeLabel, selectedRootTab == RootTab.STATUS)
        renderNavItem(navServers, navServersIcon, navServersLabel, selectedRootTab == RootTab.SERVERS)
        renderNavItem(navSettings, navSettingsIcon, navSettingsLabel, selectedRootTab == RootTab.SETTINGS)
    }

    private fun animateRootPageTransition(fromTab: RootTab, toTab: RootTab) {
        val outgoingPage = pageForRootTab(fromTab)
        val incomingPage = pageForRootTab(toTab)
        val width = findViewById<View>(R.id.mainContentFrame).width
            .takeIf { it > 0 }
            ?: resources.displayMetrics.widthPixels
        val direction = if (rootTabIndex(toTab) >= rootTabIndex(fromTab)) 1f else -1f
        val interpolator = AccelerateDecelerateInterpolator()

        outgoingPage.animate().cancel()
        incomingPage.animate().cancel()
        incomingPage.visibility = View.VISIBLE
        incomingPage.bringToFront()
        incomingPage.translationX = direction * width
        incomingPage.alpha = 1f
        outgoingPage.isEnabled = false
        incomingPage.isEnabled = false

        outgoingPage.animate()
            .translationX(-direction * width)
            .alpha(1f)
            .setDuration(ROOT_PAGE_TRANSITION_MS)
            .setInterpolator(interpolator)
            .withEndAction {
                outgoingPage.visibility = View.GONE
                outgoingPage.translationX = 0f
                outgoingPage.alpha = 1f
                outgoingPage.isEnabled = true
                incomingPage.isEnabled = true
            }
            .start()

        incomingPage.animate()
            .translationX(0f)
            .alpha(1f)
            .setDuration(ROOT_PAGE_TRANSITION_MS)
            .setInterpolator(interpolator)
            .start()
    }

    private fun rootPages(): List<Pair<RootTab, View>> {
        return listOf(
            RootTab.STATUS to statusPage,
            RootTab.SERVERS to serversPage,
            RootTab.SETTINGS to settingsPage,
            RootTab.EDITOR to configEditorPage,
            RootTab.TEST_OUTPUT to testOutputPage,
        )
    }

    private fun pageForRootTab(tab: RootTab): View {
        return when (tab) {
            RootTab.STATUS -> statusPage
            RootTab.SERVERS -> serversPage
            RootTab.SETTINGS -> settingsPage
            RootTab.EDITOR -> configEditorPage
            RootTab.TEST_OUTPUT -> testOutputPage
        }
    }

    private fun rootTabIndex(tab: RootTab): Int {
        return when (tab) {
            RootTab.STATUS -> 0
            RootTab.SERVERS -> 1
            RootTab.SETTINGS -> 2
            RootTab.EDITOR -> 3
            RootTab.TEST_OUTPUT -> 4
        }
    }

    private fun renderNavItem(
        container: LinearLayout,
        icon: ImageView,
        label: TextView,
        selected: Boolean,
    ) {
        container.background = if (selected) {
            getDrawable(R.drawable.bg_tab_selected)
        } else {
            null
        }
        val color = Color.parseColor(if (selected) "#3B82F6" else "#111827")
        icon.setColorFilter(color)
        label.setTextColor(color)
    }

    @Suppress("DEPRECATION")
    private fun configureSystemBars() {
        window.statusBarColor = Color.parseColor("#F2F2F7")
        window.navigationBarColor = Color.parseColor("#F2F2F7")
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.M) {
            var flags = View.SYSTEM_UI_FLAG_LIGHT_STATUS_BAR
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                flags = flags or View.SYSTEM_UI_FLAG_LIGHT_NAVIGATION_BAR
            }
            window.decorView.systemUiVisibility = flags
        }
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
        if (!startTunnelService(TunnelActions.ACTION_DISCONNECT)) {
            markTunnelStoppedLocally("Tunnel stop request could not reach Android VPN service")
        }
    }

    private fun startTunnelService(action: String): Boolean {
        when (action) {
            TunnelActions.ACTION_CONNECT -> {
                TunnelPreferences.updateState(this, TunnelState.CONNECTING, "Starting Android VPN service")
                TunnelLogStore.append(this, "[APP] Start requested through Android VpnService")
            }
            TunnelActions.ACTION_DISCONNECT ->
                TunnelLogStore.append(this, "[APP] Stop requested through Android VpnService")
        }
        val intent = Intent(this, TunnelVpnService::class.java).setAction(action)
        return runCatching {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                startForegroundService(intent)
            } else {
                startService(intent)
            }
        }.onFailure { error ->
            val detail = error.localizedMessage ?: error.javaClass.simpleName
            TunnelLogStore.append(this, "[APP] Android VPN service request failed: $detail")
            if (action == TunnelActions.ACTION_CONNECT) {
                TunnelPreferences.updateState(this, TunnelState.FAILED, "Android VPN service did not start: $detail")
            }
        }.isSuccess
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
            TunnelState.CONNECTING -> "Cancel"
            TunnelState.DISCONNECTING -> "Stopping"
            TunnelState.RUNNING -> "Stop"
            else -> "Connect"
        }
        startButton.isEnabled = snapshot.state != TunnelState.DISCONNECTING

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
        if (snapshot.state != TunnelState.CONNECTING && snapshot.state != TunnelState.DISCONNECTING) {
            return false
        }

        val stateAgeMs = System.currentTimeMillis() - snapshot.updatedAtMs
        return when (snapshot.state) {
            TunnelState.CONNECTING -> {
                if (stateAgeMs < CONNECT_TIMEOUT_MS) {
                    false
                } else {
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
                    true
                }
            }
            TunnelState.DISCONNECTING -> {
                if (stateAgeMs < DISCONNECT_TIMEOUT_MS) {
                    false
                } else {
                    markTunnelStoppedLocally("Tunnel stop took too long; local state was reset")
                    true
                }
            }
            else -> false
        }
    }

    private fun scheduleStateWatchdog(snapshot: TunnelSnapshot) {
        uiHandler.removeCallbacks(stalledConnectRunnable)
        if (snapshot.state != TunnelState.CONNECTING && snapshot.state != TunnelState.DISCONNECTING) {
            return
        }
        if (snapshot.updatedAtMs <= 0L) {
            return
        }

        val timeoutMs = if (snapshot.state == TunnelState.DISCONNECTING) {
            DISCONNECT_TIMEOUT_MS
        } else {
            CONNECT_TIMEOUT_MS
        }
        val remainingMs = (timeoutMs - (System.currentTimeMillis() - snapshot.updatedAtMs))
            .coerceAtLeast(250L)
        uiHandler.postDelayed(stalledConnectRunnable, remainingMs)
    }

    private fun markTunnelStoppedLocally(message: String) {
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
        TunnelPreferences.updateState(this, TunnelState.IDLE, message)
        TunnelLogStore.append(this, "[APP] $message")
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

    private fun renderServersPage() {
        refreshConfigurationState()
        val configurations = TunnelPreferences.loadSavedConfigurations(this)
        val selectedId = TunnelPreferences.loadSelectedConfigurationId(this)
        val activeId = TunnelPreferences.loadActiveConfigurationId(this)
        val snapshot = TunnelPreferences.loadSnapshot(this)
        val runtime = TunnelPreferences.loadRuntimeSnapshot(this)
        val showActive = shouldShowActiveConfiguration(snapshot, runtime)

        // Update Edit/Done button state
        val isDarkMode = resources.configuration.uiMode and android.content.res.Configuration.UI_MODE_NIGHT_MASK ==
            android.content.res.Configuration.UI_MODE_NIGHT_YES
        val normalButtonColor = if (isDarkMode) Color.parseColor("#1C1C1E") else Color.WHITE
        val normalButtonTextColor = if (isDarkMode) Color.WHITE else Color.parseColor("#111827")
        val editBg = GradientDrawable().apply {
            shape = GradientDrawable.RECTANGLE
            cornerRadius = dpToPx(999f)
            setColor(if (serversEditMode) Color.parseColor("#2563EB") else normalButtonColor)
        }
        serversEditButton.background = editBg
        serversEditButton.text = if (serversEditMode) "Done" else "Edit"
        serversEditButton.setTextColor(if (serversEditMode) Color.WHITE else normalButtonTextColor)

        val addBg = GradientDrawable().apply {
            shape = GradientDrawable.RECTANGLE
            cornerRadius = dpToPx(999f)
            setColor(normalButtonColor)
        }
        serversAddButton.background = addBg
        serversAddButton.setColorFilter(normalButtonTextColor)

        serversEmptyState.visibility = if (configurations.isEmpty()) View.VISIBLE else View.GONE
        serversListContainer.removeAllViews()

        val inflater = LayoutInflater.from(this)
        configurations.forEachIndexed { index, savedConfiguration ->
            val row = inflater.inflate(
                R.layout.item_saved_configuration,
                serversListContainer,
                false,
            )
            val root = row.findViewById<LinearLayout>(R.id.configRowRoot)
            root.forceLeftToRightTree()
            val title = row.findViewById<TextView>(R.id.configRowTitle)
            val subtitle = row.findViewById<TextView>(R.id.configRowSubtitle)
            val selectedIcon = row.findViewById<ImageView>(R.id.configRowSelectedIcon)
            val deleteButton = row.findViewById<TextView>(R.id.configRowDeleteButton)
            val selectedBadge = row.findViewById<TextView>(R.id.configRowSelectedBadge)
            val activeBadge = row.findViewById<TextView>(R.id.configRowActiveBadge)
            val editButton = row.findViewById<ImageButton>(R.id.configRowEditButton)

            val isSelected = selectedId == savedConfiguration.id
            val isActive = showActive && activeId == savedConfiguration.id

            title.text = savedConfiguration.displayName
            subtitle.text = buildRowSubtitle(savedConfiguration, isSelected)

            // Edit mode visibility
            deleteButton.visibility = if (serversEditMode) View.VISIBLE else View.GONE
            selectedIcon.visibility = if (!serversEditMode && isSelected) View.VISIBLE else View.GONE
            selectedIcon.setImageResource(if (isSelected) R.drawable.ic_check_circle else R.drawable.ic_circle_outline)
            editButton.visibility = if (serversEditMode) View.GONE else View.VISIBLE

            selectedBadge.visibility = View.GONE
            activeBadge.visibility = if (isActive) View.VISIBLE else View.GONE
            if (isActive) {
                applyBadgeStyle(
                    activeBadge,
                    textColor = Color.parseColor("#15803D"),
                    strokeColor = Color.parseColor("#86EFAC"),
                )
            }
            applyServerRowStyle(root)
            editButton.setColorFilter(Color.parseColor("#111827"))

            deleteButton.setOnClickListener {
                deleteConfiguration(root, savedConfiguration.id)
            }

            root.setOnClickListener {
                if (!serversEditMode) {
                    if (TunnelPreferences.selectConfiguration(this, savedConfiguration.id)) {
                        refreshConfigurationState()
                        renderServersPage()
                        renderSettingsPage()
                        renderConfigurationSummary()
                        renderDashboard()
                    }
                }
            }

            editButton.setOnClickListener {
                showConfigurationEditor(existing = savedConfiguration)
            }

            if (!serversEditMode) {
                addSwipeToDelete(root, savedConfiguration.id)
            } else {
                root.setOnTouchListener(null)
            }

            serversListContainer.addView(
                row,
                LinearLayout.LayoutParams(
                    ViewGroup.LayoutParams.MATCH_PARENT,
                    ViewGroup.LayoutParams.WRAP_CONTENT,
                ).apply {
                    if (index > 0) {
                        topMargin = dpInt(12f)
                    }
                },
            )
        }
    }

    private fun renderSettingsPage() {
        refreshConfigurationState()
        val current = currentConfiguration
        settingsProfilesSummaryText.text = TunnelPreferences.loadSavedConfigurations(this).size.toString()
        settingsProtocolSummaryText.text = displayStackMode(current.stackMode)
        settingsTransportSummaryText.text = if (current.usesCustomCarrier || current.usesPacketChain || current.usesPrivateRelay) {
            current.ingressLabel
        } else {
            displayTransportMode(current.transportMode)
        }
        settingsFragmentationSummaryText.text = if (current.fragmentEnabled) {
            "${current.fragmentSizeValue} bytes"
        } else {
            "Off"
        }
        settingsAboutVersionText.text = packageVersionLabel()
    }

    private fun buildRowSubtitle(savedConfiguration: SavedTunnelConfiguration, isSelected: Boolean): String {
        val configuration = savedConfiguration.configuration
        val endpoint = if (configuration.usesCustomCarrier || configuration.usesPacketChain) {
            configuration.normalizedTrojanCarrierUri
        } else if (configuration.usesPrivateRelay) {
            configuration.normalizedServerUrl
        } else {
            configuration.normalizedServerUrl
        }.ifBlank {
            savedConfiguration.subtitle
        }
        val prefix = if (isSelected) "Selected" else configuration.transportLabel
        return listOf(prefix, endpoint)
            .filter { it.isNotBlank() }
            .joinToString(separator = " · ")
    }

    private fun displayStackMode(mode: TunnelStackMode): String {
        return when (mode) {
            TunnelStackMode.PACKET_NATIVE -> "Direct Packet"
            TunnelStackMode.CUSTOM_TROJAN_CARRIER -> "DirectSock"
            TunnelStackMode.PACKET_CHAIN -> "Packet Chain"
            TunnelStackMode.PRIVATE_RELAY -> "Private Relay"
        }
    }

    private fun displayTransportMode(mode: TunnelTransportMode): String {
        return when (mode) {
            TunnelTransportMode.AUTO -> "Auto"
            TunnelTransportMode.WEBSOCKET -> "WebSocket"
            TunnelTransportMode.HTTP -> "HTTP"
            TunnelTransportMode.STEALTH -> "Stealth"
            TunnelTransportMode.OBFS -> "Obfs"
            TunnelTransportMode.MEEK -> "Meek"
            TunnelTransportMode.QUIC -> "QUIC"
        }
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

        val pingValue = runtime.egressPingMs ?: runtime.lastPingMs ?: diagnostics.endpointLatencyMs
        metricPingValue.text = pingValue?.let { "$it ms" } ?: "--"
        metricPingDetail.text = when {
            runtime.egressPingMs != null -> "Internet probe"
            runtime.lastPingMs != null -> "Transport round-trip"
            diagnostics.endpointLatencyMs != null -> "TCP probe"
            else -> "Not measured"
        }

        metricCountryValue.text = if (runtime.tunnelActive) {
            runtime.serverCountryName ?: runtime.serverCountryCode ?: "Unknown"
        } else {
            "--"
        }
        metricCountryDetail.text = runtime.egressTarget?.let { "Probe: $it" }
            ?: if (runtime.tunnelActive) "Probe country unavailable" else "Waiting for probe"

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
            if (selectedRootTab == RootTab.TEST_OUTPUT) {
                renderTestOutput()
            }
            return
        }

        if (logsCollapsed) {
            logsMetaText.text = "${logs.size} lines hidden"
            logsView.text = ""
            if (selectedRootTab == RootTab.TEST_OUTPUT) {
                renderTestOutput()
            }
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
        if (selectedRootTab == RootTab.TEST_OUTPUT) {
            renderTestOutput()
        }
    }

    private fun openTestOutputPage(runImmediately: Boolean) {
        testOutputReturnTab = when (selectedRootTab) {
            RootTab.TEST_OUTPUT -> testOutputReturnTab
            RootTab.EDITOR -> RootTab.SERVERS
            else -> selectedRootTab
        }
        renderTestOutput()
        selectRootTab(RootTab.TEST_OUTPUT)
        if (runImmediately) {
            runTestOutput()
        }
    }

    private fun runTestOutput() {
        PacketBridge.emitTestOutput()
        TunnelLogStore.append(this, "[APP] Requested Rust test output")
        renderTestOutput()
        uiHandler.postDelayed({ renderTestOutput() }, 350L)
    }

    private fun renderTestOutput() {
        val logs = TunnelLogStore.load(this)
        val output = if (logs.isEmpty()) {
            "No test output yet."
        } else {
            logs.takeLast(MAX_TEST_OUTPUT_LINES)
                .joinToString(separator = "\n") { formatLogLineForDisplay(it) }
        }
        testOutputText.text = output
        testOutputScrollView.post {
            testOutputScrollView.scrollTo(0, testOutputText.bottom)
        }
    }

    private fun copyTestOutputToClipboard() {
        val output = testOutputText.text?.toString().orEmpty()
        if (output.isBlank() || output == "No test output yet.") {
            Toast.makeText(this, "No test output to copy", Toast.LENGTH_SHORT).show()
            return
        }

        val clipboard = getSystemService(ClipboardManager::class.java)
        clipboard.setPrimaryClip(ClipData.newPlainText("Packet Test Output", output))
        Toast.makeText(this, "Test output copied", Toast.LENGTH_SHORT).show()
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
        val isProtected = snapshot.state == TunnelState.RUNNING && runtime.tunnelActive
        val configuration = displayConfiguration(snapshot, runtime)
        
        statusBadgeText.text = when (snapshot.state) {
            TunnelState.RUNNING -> if (runtime.tunnelActive) "CONNECTED" else "VERIFYING"
            TunnelState.CONNECTING -> "CONNECTING"
            TunnelState.REQUESTING_PERMISSION -> "APPROVAL"
            TunnelState.DISCONNECTING -> "STOPPING"
            TunnelState.FAILED -> "FAILED"
            TunnelState.IDLE -> "DISCONNECTED"
        }
        statusBadgeText.setTextColor(palette.accent)

        if (isProtected && runtime.connectedSince != null) {
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

    private fun showConfigurationEditor(existing: SavedTunnelConfiguration?) {
        val editorSeedConfiguration = existing?.configuration ?: TunnelConfiguration()
        editorExisting = existing
        editorStackMode = editorSeedConfiguration.stackMode
        editorTransportMode = editorSeedConfiguration.transportMode
        editorTitleText.text = if (existing == null) "New Server" else "Edit Server"
        editorErrorText.visibility = View.GONE
        editorNameInput.hint = editorSeedConfiguration.suggestedName
        editorNameInput.setText(existing?.name.orEmpty())
        editorServerUrlInput.setText(editorSeedConfiguration.serverUrl)
        editorSecretInput.setText(editorSeedConfiguration.secret)
        editorListenPortInput.setText(editorSeedConfiguration.listenPort)
        editorCdnEdgeInput.setText(editorSeedConfiguration.cdnEdge)
        editorHostOverrideInput.setText(editorSeedConfiguration.hostOverride)
        editorSniOverrideInput.setText(editorSeedConfiguration.sniOverride)
        editorObfsKeyInput.setText(editorSeedConfiguration.obfsKey)
        editorUpstreamProxyInput.setText(editorSeedConfiguration.upstreamProxy)
        editorTrojanUriInput.setText(editorSeedConfiguration.trojanCarrierUri)
        editorCarrierPortInput.setText(editorSeedConfiguration.carrierProxyPort)
        editorFragmentSwitch.isChecked = editorSeedConfiguration.fragmentEnabled
        applyFragmentSwitchStyle()
        editorFragmentSizeInput.setText(editorSeedConfiguration.fragmentSize)
        renderEditorMode()
        selectRootTab(RootTab.EDITOR)
    }

    private fun renderEditorMode() {
        val isDirectSock = editorStackMode == TunnelStackMode.CUSTOM_TROJAN_CARRIER
        val isPacketChain = editorStackMode == TunnelStackMode.PACKET_CHAIN
        val isPrivateRelay = editorStackMode == TunnelStackMode.PRIVATE_RELAY
        editorStackValue.text = displayStackMode(editorStackMode)
        editorPacketNativeSection.visibility = if (isDirectSock) View.GONE else View.VISIBLE
        editorDirectSockSection.visibility = if (isDirectSock || isPacketChain) View.VISIBLE else View.GONE
        editorTransportRow.text = when {
            isPacketChain -> "Transport      Auto through DirectSock"
            isPrivateRelay -> "Transport      Private WebSocket relay"
            else -> "Transport      ${displayTransportMode(editorTransportMode)}"
        }
    }

    private fun saveConfigurationEditor() {
        val existing = editorExisting
        val updatedConfiguration = TunnelConfiguration(
            stackMode = editorStackMode,
            serverUrl = editorServerUrlInput.text.toString(),
            secret = editorSecretInput.text.toString(),
            listenPort = editorListenPortInput.text.toString(),
            cdnEdge = editorCdnEdgeInput.text.toString(),
            hostOverride = editorHostOverrideInput.text.toString(),
            sniOverride = editorSniOverrideInput.text.toString(),
            transportMode = editorTransportMode,
            obfsKey = editorObfsKeyInput.text.toString(),
            upstreamProxy = editorUpstreamProxyInput.text.toString(),
            fragmentEnabled = editorFragmentSwitch.isChecked,
            fragmentSize = editorFragmentSizeInput.text.toString(),
            trojanCarrierUri = editorTrojanUriInput.text.toString(),
            carrierProxyPort = editorCarrierPortInput.text.toString(),
        )

        val validationError = updatedConfiguration.validationError
        if (validationError != null) {
            editorErrorText.text = validationError
            editorErrorText.visibility = View.VISIBLE
            return
        }

        val saved = if (existing == null) {
            TunnelPreferences.addConfiguration(
                context = this,
                name = editorNameInput.text.toString(),
                configuration = updatedConfiguration,
            )
        } else {
            TunnelPreferences.updateConfiguration(
                context = this,
                id = existing.id,
                name = editorNameInput.text.toString(),
                configuration = updatedConfiguration,
            )
        }

        if (saved == null) {
            editorErrorText.text = "Unable to save configuration."
            editorErrorText.visibility = View.VISIBLE
            return
        }

        refreshConfigurationState()
        renderServersPage()
        renderSettingsPage()
        renderConfigurationSummary()
        renderDashboard()
        selectRootTab(RootTab.SERVERS)
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

    private fun toggleServersEditMode() {
        serversEditMode = !serversEditMode
        renderServersPage()
    }

    private fun showStackModeMenu(@Suppress("UNUSED_PARAMETER") anchor: View) {
        val popup = PopupMenu(this, editorStackValue, android.view.Gravity.END)
        TunnelStackMode.values().forEachIndexed { index, mode ->
            popup.menu.add(0, index, index, mode.title)
        }
        popup.setOnMenuItemClickListener { item ->
            val mode = TunnelStackMode.values().getOrNull(item.itemId)
            if (mode != null) {
                editorStackMode = mode
                renderEditorMode()
            }
            true
        }
        popup.show()
    }

    @Suppress("DEPRECATION")
    private fun applyFragmentSwitchStyle() {
        editorFragmentSwitch.trackDrawable = getDrawable(R.drawable.bg_switch_track)
        editorFragmentSwitch.thumbTintList = ColorStateList.valueOf(Color.WHITE)
    }

    private fun deleteConfiguration(itemView: View, configurationId: String) {
        itemView.animate()
            .translationX(-itemView.width.toFloat())
            .alpha(0f)
            .setDuration(220)
            .withEndAction {
                TunnelPreferences.removeConfiguration(this, configurationId)
                refreshConfigurationState()
                renderServersPage()
                renderSettingsPage()
                renderConfigurationSummary()
            }
            .start()
    }

    private fun addSwipeToDelete(itemView: View, configurationId: String) {
        var startX = 0f
        var startY = 0f
        var swiping = false
        val threshold = dpToPx(72f)

        itemView.setOnTouchListener { view, event ->
            when (event.actionMasked) {
                MotionEvent.ACTION_DOWN -> {
                    startX = event.rawX
                    startY = event.rawY
                    swiping = false
                    false
                }
                MotionEvent.ACTION_MOVE -> {
                    val dx = event.rawX - startX
                    val dy = event.rawY - startY
                    if (!swiping && kotlin.math.abs(dx) > 20 && kotlin.math.abs(dx) > kotlin.math.abs(dy) * 1.5f) {
                        swiping = true
                        view.parent?.requestDisallowInterceptTouchEvent(true)
                    }
                    if (swiping && dx < 0) {
                        view.translationX = dx.coerceAtLeast(-view.width.toFloat() * 0.7f)
                        true
                    } else {
                        false
                    }
                }
                MotionEvent.ACTION_UP -> {
                    if (swiping) {
                        swiping = false
                        view.parent?.requestDisallowInterceptTouchEvent(false)
                        val tx = view.translationX
                        if (tx < -threshold) {
                            deleteConfiguration(view, configurationId)
                        } else {
                            view.animate().translationX(0f).alpha(1f).setDuration(200).start()
                        }
                        true
                    } else {
                        false
                    }
                }
                MotionEvent.ACTION_CANCEL -> {
                    if (swiping) {
                        swiping = false
                        view.parent?.requestDisallowInterceptTouchEvent(false)
                        view.animate().translationX(0f).alpha(1f).setDuration(200).start()
                    }
                    false
                }
                else -> false
            }
        }
    }

    private fun applyServerRowStyle(view: View) {
        view.background = GradientDrawable().apply {
            shape = GradientDrawable.RECTANGLE
            cornerRadius = dpInt(24f).toFloat()
            setColor(Color.WHITE)
        }
    }

    private fun applyBadgeStyle(
        textView: TextView,
        textColor: Int,
        strokeColor: Int,
    ) {
        textView.setTextColor(textColor)
        textView.background = GradientDrawable().apply {
            shape = GradientDrawable.RECTANGLE
            cornerRadius = dpInt(999f).toFloat()
            setColor(Color.TRANSPARENT)
            setStroke(dpInt(1f), strokeColor)
        }
    }

    private fun dpInt(valueDp: Float): Int {
        return dpToPx(valueDp).toInt().coerceAtLeast(1)
    }

    private fun openUrl(url: String) {
        runCatching {
            startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(url)))
        }
    }

    private fun packageVersionLabel(): String {
        return runCatching {
            val pkg = packageManager.getPackageInfo(packageName, 0)
            val versionCode = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
                pkg.longVersionCode.toString()
            } else {
                @Suppress("DEPRECATION")
                pkg.versionCode.toString()
            }
            "${pkg.versionName ?: "1.0"} ($versionCode)"
        }.getOrDefault("1.0")
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

    /**
     * Copy a full status snapshot (state, error, configuration, runtime,
     * recent log tail) to the clipboard so the user can share it directly
     * from the main status screen. Reuses the same builder as the Logs
     * page so the report format stays consistent.
     */
    private fun copyStatusDetails() {
        val clipboard = getSystemService(ClipboardManager::class.java)
        val report = TunnelDiagnosticReport.build(this)
        clipboard.setPrimaryClip(ClipData.newPlainText("Packet Status", report))
        Toast.makeText(this, "Status copied", Toast.LENGTH_SHORT).show()
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
        const val CONNECT_TIMEOUT_MS = 330_000L
        const val DISCONNECT_TIMEOUT_MS = 8_000L
        const val LOG_RENDER_DEBOUNCE_MS = 150L
        const val MAX_VISIBLE_LOG_LINES = 250
        const val MAX_TEST_OUTPUT_LINES = 250
        const val ROOT_PAGE_TRANSITION_MS = 230L
        const val PRIVACY_URL = "https://packet-tunnels.github.io/packet-public/privacy.html"
        const val TERMS_URL = "https://packet-tunnels.github.io/packet-public/terms.html"
        const val SUPPORT_URL = "https://packet-tunnels.github.io/packet-public/support.html"
    }
}
