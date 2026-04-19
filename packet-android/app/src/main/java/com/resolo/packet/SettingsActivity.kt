package com.resolo.packet

import android.app.Activity
import android.app.Dialog
import android.graphics.Color
import android.graphics.drawable.ColorDrawable
import android.graphics.drawable.GradientDrawable
import android.os.Bundle
import android.text.method.HideReturnsTransformationMethod
import android.text.method.PasswordTransformationMethod
import android.view.LayoutInflater
import android.view.View
import android.view.ViewGroup
import android.view.WindowManager
import android.widget.ArrayAdapter
import android.widget.Button
import android.widget.EditText
import android.widget.ImageButton
import android.widget.LinearLayout
import android.widget.Spinner
import android.widget.TextView

class SettingsActivity : Activity() {
    private lateinit var backButton: ImageButton
    private lateinit var addButton: ImageButton
    private lateinit var configListContainer: LinearLayout
    private lateinit var emptyStateContainer: View
    private lateinit var reconnectNote: View
    private lateinit var disclosureStatusBadge: TextView
    private lateinit var reviewDisclosureButton: Button

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(R.layout.activity_settings)

        backButton = findViewById(R.id.settingsBackButton)
        addButton = findViewById(R.id.settingsAddButton)
        configListContainer = findViewById(R.id.settingsConfigListContainer)
        emptyStateContainer = findViewById(R.id.settingsEmptyStateContainer)
        reconnectNote = findViewById(R.id.settingsReconnectNote)
        disclosureStatusBadge = findViewById(R.id.settingsDisclosureStatusBadge)
        reviewDisclosureButton = findViewById(R.id.settingsReviewDisclosureButton)

        backButton.setOnClickListener { finish() }
        addButton.setOnClickListener { showConfigurationEditor(existing = null) }
        reviewDisclosureButton.setOnClickListener { showDisclosureReview() }

        renderScreen()
    }

    override fun onStart() {
        super.onStart()
        renderScreen()
    }

    override fun onResume() {
        super.onResume()
        renderScreen()
    }

    private fun renderScreen() {
        val configurations = TunnelPreferences.loadSavedConfigurations(this)
        val selectedId = TunnelPreferences.loadSelectedConfigurationId(this)
        val activeId = TunnelPreferences.loadActiveConfigurationId(this)
        val snapshot = TunnelPreferences.loadSnapshot(this)
        val runtime = TunnelPreferences.loadRuntimeSnapshot(this)
        val showActive = runtime.tunnelActive ||
            snapshot.state == TunnelState.RUNNING ||
            snapshot.state == TunnelState.CONNECTING ||
            snapshot.state == TunnelState.DISCONNECTING

        emptyStateContainer.visibility = if (configurations.isEmpty()) View.VISIBLE else View.GONE
        renderConfigurationList(
            configurations = configurations,
            selectedId = selectedId,
            activeId = activeId,
            showActive = showActive,
        )

        reconnectNote.visibility = if (
            showActive &&
            !selectedId.isNullOrBlank() &&
            !activeId.isNullOrBlank() &&
            selectedId != activeId
        ) {
            View.VISIBLE
        } else {
            View.GONE
        }

        val disclosureAcknowledged = TunnelPreferences.isVpnDisclosureAcknowledged(this)
        disclosureStatusBadge.text = if (disclosureAcknowledged) "Accepted" else "Required"
        applyBadgeStyle(
            disclosureStatusBadge,
            textColor = Color.parseColor(if (disclosureAcknowledged) "#15803D" else "#B45309"),
            strokeColor = Color.parseColor(if (disclosureAcknowledged) "#15803D" else "#F59E0B"),
        )
    }

    private fun renderConfigurationList(
        configurations: List<SavedTunnelConfiguration>,
        selectedId: String?,
        activeId: String?,
        showActive: Boolean,
    ) {
        configListContainer.removeAllViews()
        val inflater = LayoutInflater.from(this)

        configurations.forEachIndexed { index, savedConfiguration ->
            val row = inflater.inflate(
                R.layout.item_saved_configuration,
                configListContainer,
                false,
            )
            val root = row.findViewById<View>(R.id.configRowRoot)
            val title = row.findViewById<TextView>(R.id.configRowTitle)
            val subtitle = row.findViewById<TextView>(R.id.configRowSubtitle)
            val selectedBadge = row.findViewById<TextView>(R.id.configRowSelectedBadge)
            val activeBadge = row.findViewById<TextView>(R.id.configRowActiveBadge)
            val editButton = row.findViewById<ImageButton>(R.id.configRowEditButton)

            val isSelected = selectedId == savedConfiguration.id
            val isActive = showActive && activeId == savedConfiguration.id
            title.text = savedConfiguration.displayName
            subtitle.text = buildRowSubtitle(savedConfiguration)

            selectedBadge.visibility = if (isSelected) View.VISIBLE else View.GONE
            activeBadge.visibility = if (isActive) View.VISIBLE else View.GONE

            if (isSelected) {
                applyBadgeStyle(
                    selectedBadge,
                    textColor = Color.parseColor("#2563EB"),
                    strokeColor = Color.parseColor("#93C5FD"),
                )
            }

            if (isActive) {
                applyBadgeStyle(
                    activeBadge,
                    textColor = Color.parseColor("#15803D"),
                    strokeColor = Color.parseColor("#86EFAC"),
                )
            }

            applyRowStyle(root, isSelected = isSelected, isActive = isActive)
            editButton.setColorFilter(
                Color.parseColor(
                    if (isActive) "#15803D" else if (isSelected) "#2563EB" else "#6B7280"
                )
            )

            root.setOnClickListener {
                if (TunnelPreferences.selectConfiguration(this, savedConfiguration.id)) {
                    renderScreen()
                }
            }

            editButton.setOnClickListener {
                showConfigurationEditor(existing = savedConfiguration)
            }

            configListContainer.addView(
                row,
                LinearLayout.LayoutParams(
                    ViewGroup.LayoutParams.MATCH_PARENT,
                    ViewGroup.LayoutParams.WRAP_CONTENT,
                ).apply {
                    if (index > 0) {
                        topMargin = dpToPx(12f)
                    }
                },
            )
        }
    }

    private fun buildRowSubtitle(savedConfiguration: SavedTunnelConfiguration): String {
        return buildList {
            savedConfiguration.configuration.normalizedServerUrl
                .takeIf { it.isNotBlank() }
                ?.let(::add)
            add(savedConfiguration.configuration.transportLabel)
            if (savedConfiguration.configuration.usesCdn) {
                add(savedConfiguration.configuration.ingressLabel)
            }
        }.ifEmpty {
            listOf(savedConfiguration.subtitle)
        }.joinToString(separator = " · ")
    }

    private fun showConfigurationEditor(existing: SavedTunnelConfiguration?) {
        val dialog = Dialog(this)
        dialog.setContentView(R.layout.dialog_settings)
        dialog.window?.setBackgroundDrawable(ColorDrawable(Color.TRANSPARENT))
        dialog.window?.setLayout(
            WindowManager.LayoutParams.MATCH_PARENT,
            WindowManager.LayoutParams.WRAP_CONTENT,
        )

        val seedConfiguration = existing?.configuration ?: TunnelConfiguration()
        val transportModes = TunnelTransportMode.values().toList()

        val titleView = dialog.findViewById<TextView>(R.id.settingsDialogTitle)
        val closeButton = dialog.findViewById<ImageButton>(R.id.settingsCloseButton)
        val errorText = dialog.findViewById<TextView>(R.id.settingsErrorText)
        val nameInput = dialog.findViewById<EditText>(R.id.settingsConfigNameInput)
        val serverUrlInput = dialog.findViewById<EditText>(R.id.settingsServerUrlInput)
        val secretInput = dialog.findViewById<EditText>(R.id.settingsSecretInput)
        val secretToggle = dialog.findViewById<ImageButton>(R.id.secretToggleVisibility)
        val listenPortInput = dialog.findViewById<EditText>(R.id.settingsListenPortInput)
        val transportSpinner = dialog.findViewById<Spinner>(R.id.settingsTransportSpinner)
        val cdnEdgeInput = dialog.findViewById<EditText>(R.id.settingsCdnEdgeInput)
        val hostOverrideInput = dialog.findViewById<EditText>(R.id.settingsHostOverrideInput)
        val sniOverrideInput = dialog.findViewById<EditText>(R.id.settingsSniOverrideInput)
        val fragmentSwitch = dialog.findViewById<android.widget.Switch>(R.id.settingsFragmentSwitch)
        val fragmentSizeInput = dialog.findViewById<EditText>(R.id.settingsFragmentSizeInput)
        val cancelButton = dialog.findViewById<Button>(R.id.settingsCancelButton)
        val saveButton = dialog.findViewById<Button>(R.id.settingsSaveButton)

        titleView.text = if (existing == null) "New Server" else "Edit Server"
        nameInput.hint = seedConfiguration.suggestedName
        nameInput.setText(existing?.name.orEmpty())
        serverUrlInput.setText(seedConfiguration.serverUrl)
        secretInput.setText(seedConfiguration.secret)
        listenPortInput.setText(seedConfiguration.listenPort)
        cdnEdgeInput.setText(seedConfiguration.cdnEdge)
        hostOverrideInput.setText(seedConfiguration.hostOverride)
        sniOverrideInput.setText(seedConfiguration.sniOverride)
        fragmentSwitch.isChecked = seedConfiguration.fragmentEnabled
        fragmentSizeInput.setText(seedConfiguration.fragmentSize)

        transportSpinner.adapter = ArrayAdapter(
            this,
            android.R.layout.simple_spinner_item,
            transportModes.map { it.title },
        ).also { adapter ->
            adapter.setDropDownViewResource(android.R.layout.simple_spinner_dropdown_item)
        }
        transportSpinner.setSelection(
            transportModes.indexOf(seedConfiguration.transportMode).coerceAtLeast(0)
        )

        var secretVisible = false
        fun updateSecretVisibility() {
            val selection = secretInput.selectionEnd.coerceAtLeast(0)
            secretInput.transformationMethod = if (secretVisible) {
                HideReturnsTransformationMethod.getInstance()
            } else {
                PasswordTransformationMethod.getInstance()
            }
            secretToggle.setImageResource(
                if (secretVisible) R.drawable.ic_visibility_off else R.drawable.ic_visibility
            )
            secretInput.setSelection(selection.coerceAtMost(secretInput.text.length))
        }

        updateSecretVisibility()

        secretToggle.setOnClickListener {
            secretVisible = !secretVisible
            updateSecretVisibility()
        }

        closeButton.setOnClickListener { dialog.dismiss() }
        cancelButton.setOnClickListener { dialog.dismiss() }
        saveButton.setOnClickListener {
            val updatedConfiguration = TunnelConfiguration(
                serverUrl = serverUrlInput.text.toString(),
                secret = secretInput.text.toString(),
                listenPort = listenPortInput.text.toString(),
                cdnEdge = cdnEdgeInput.text.toString(),
                hostOverride = hostOverrideInput.text.toString(),
                sniOverride = sniOverrideInput.text.toString(),
                transportMode = transportModes.getOrElse(transportSpinner.selectedItemPosition) {
                    TunnelTransportMode.AUTO
                },
                fragmentEnabled = fragmentSwitch.isChecked,
                fragmentSize = fragmentSizeInput.text.toString(),
            )

            val validationError = updatedConfiguration.validationError
            if (validationError != null) {
                errorText.text = validationError
                errorText.visibility = View.VISIBLE
                return@setOnClickListener
            }

            val saved = if (existing == null) {
                TunnelPreferences.addConfiguration(
                    context = this,
                    name = nameInput.text.toString(),
                    configuration = updatedConfiguration,
                )
            } else {
                TunnelPreferences.updateConfiguration(
                    context = this,
                    id = existing.id,
                    name = nameInput.text.toString(),
                    configuration = updatedConfiguration,
                )
            }

            if (saved == null) {
                errorText.text = "Unable to save configuration."
                errorText.visibility = View.VISIBLE
                return@setOnClickListener
            }

            errorText.visibility = View.GONE
            dialog.dismiss()
            renderScreen()
        }

        dialog.show()
    }

    private fun showDisclosureReview() {
        val acknowledged = TunnelPreferences.isVpnDisclosureAcknowledged(this)
        VpnDisclosureDialogs.show(
            activity = this,
            acceptTitle = if (acknowledged) "Done" else "Acknowledge",
            dismissTitle = "Close",
            onAccept = { renderScreen() },
            onDismiss = { renderScreen() },
        )
    }

    private fun applyRowStyle(
        view: View,
        isSelected: Boolean,
        isActive: Boolean,
    ) {
        val strokeColor = when {
            isActive -> Color.parseColor("#16A34A")
            isSelected -> Color.parseColor("#2563EB")
            else -> Color.parseColor("#D1D5DB")
        }
        view.background = GradientDrawable().apply {
            shape = GradientDrawable.RECTANGLE
            cornerRadius = dpToPx(18f).toFloat()
            setColor(Color.TRANSPARENT)
            setStroke(dpToPx(1f), strokeColor)
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
            cornerRadius = dpToPx(999f).toFloat()
            setColor(Color.TRANSPARENT)
            setStroke(dpToPx(1f), strokeColor)
        }
    }

    private fun dpToPx(valueDp: Float): Int {
        return (valueDp * resources.displayMetrics.density).toInt().coerceAtLeast(1)
    }
}
