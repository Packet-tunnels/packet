package com.resolo.packet

import android.content.Context
import android.content.Intent
import android.content.SharedPreferences
import android.os.Handler
import android.os.Looper
import org.json.JSONArray
import org.json.JSONObject

object TunnelEvents {
    private const val LOG_BROADCAST_DEBOUNCE_MS = 120L
    private val mainHandler = Handler(Looper.getMainLooper())
    private var pendingLogBroadcast: Runnable? = null

    fun broadcastState(context: Context, stateName: String, message: String) {
        val intent = Intent(TunnelActions.ACTION_STATE_UPDATED)
            .setPackage(context.packageName)
            .putExtra("state", stateName)
            .putExtra("message", message)
        context.sendBroadcast(intent)
    }

    fun broadcastLog(context: Context) {
        val appContext = context.applicationContext
        val runnable = synchronized(this) {
            pendingLogBroadcast?.let { return }

            Runnable {
                synchronized(this) {
                    pendingLogBroadcast = null
                }
                appContext.sendBroadcast(Intent(TunnelActions.ACTION_LOG_UPDATED).setPackage(appContext.packageName))
            }.also { pendingLogBroadcast = it }
        }

        mainHandler.postDelayed(runnable, LOG_BROADCAST_DEBOUNCE_MS)
    }

    fun broadcastDashboard(context: Context, runtimeJson: String?, diagJson: String?) {
        val intent = Intent(TunnelActions.ACTION_DASHBOARD_UPDATED)
            .setPackage(context.packageName)
            .putExtra("runtime", runtimeJson)
            .putExtra("diagnostics", diagJson)
        context.sendBroadcast(intent)
    }
}

object TunnelPreferences {
    private const val PREFS_NAME = "packet_preferences"

    private const val KEY_SAVED_CONFIGURATIONS = "saved_configurations"
    private const val KEY_SELECTED_CONFIGURATION_ID = "selected_configuration_id"
    private const val KEY_ACTIVE_CONFIGURATION_ID = "active_configuration_id"
    private const val KEY_ACTIVE_CONFIGURATION_JSON = "active_configuration_json"
    private const val KEY_VPN_DISCLOSURE_ACKNOWLEDGED = "vpn_disclosure_acknowledged"

    private const val KEY_SERVER_URL = "server_url"
    private const val KEY_SECRET = "secret"
    private const val KEY_LISTEN_PORT = "listen_port"
    private const val KEY_CDN_EDGE = "cdn_edge"
    private const val KEY_HOST_OVERRIDE = "host_override"
    private const val KEY_SNI_OVERRIDE = "sni_override"
    private const val KEY_TRANSPORT_MODE = "transport_mode"
    private const val KEY_FRAGMENT_ENABLED = "fragment_enabled"
    private const val KEY_FRAGMENT_SIZE = "fragment_size"
    private const val KEY_STATE = "state"
    private const val KEY_MESSAGE = "message"
    private const val KEY_STATE_UPDATED_AT = "state_updated_at"
    private const val KEY_RUNTIME_JSON = "runtime_json"
    private const val KEY_DIAGNOSTICS_JSON = "diagnostics_json"

    private fun prefs(context: Context) =
        context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)

    private fun ensureConfigurationStore(context: Context) {
        val preferences = prefs(context)
        val savedConfigurations = loadSavedConfigurationsInternal(preferences)
        if (savedConfigurations.isNotEmpty()) {
            val selectedId = preferences.getString(KEY_SELECTED_CONFIGURATION_ID, null)
            if (selectedId == null || savedConfigurations.none { it.id == selectedId }) {
                preferences.edit()
                    .putString(KEY_SELECTED_CONFIGURATION_ID, savedConfigurations.first().id)
                    .apply()
                mirrorLegacyConfiguration(preferences.edit(), savedConfigurations.first().configuration).apply()
            }
            return
        }

        val legacyConfiguration = loadLegacyConfiguration(preferences)
        if (legacyConfiguration.isEmpty) {
            return
        }

        val importedConfiguration = SavedTunnelConfiguration(
            name = legacyConfiguration.suggestedName,
            configuration = legacyConfiguration,
        )
        preferences.edit()
            .putString(KEY_SAVED_CONFIGURATIONS, JSONArray().put(importedConfiguration.toJsonObject()).toString())
            .putString(KEY_SELECTED_CONFIGURATION_ID, importedConfiguration.id)
            .apply()
    }

    private fun loadLegacyConfiguration(preferences: SharedPreferences): TunnelConfiguration {
        val defaults = TunnelConfiguration()
        return TunnelConfiguration(
            serverUrl = preferences.getString(KEY_SERVER_URL, defaults.serverUrl) ?: defaults.serverUrl,
            secret = preferences.getString(KEY_SECRET, defaults.secret) ?: defaults.secret,
            listenPort = preferences.getString(KEY_LISTEN_PORT, defaults.listenPort) ?: defaults.listenPort,
            cdnEdge = preferences.getString(KEY_CDN_EDGE, defaults.cdnEdge) ?: defaults.cdnEdge,
            hostOverride = preferences.getString(KEY_HOST_OVERRIDE, defaults.hostOverride) ?: defaults.hostOverride,
            sniOverride = preferences.getString(KEY_SNI_OVERRIDE, defaults.sniOverride) ?: defaults.sniOverride,
            transportMode = TunnelTransportMode.fromRawValue(
                preferences.getInt(KEY_TRANSPORT_MODE, defaults.transportMode.rawValue)
            ),
            fragmentEnabled = preferences.getBoolean(KEY_FRAGMENT_ENABLED, defaults.fragmentEnabled),
            fragmentSize = preferences.getString(KEY_FRAGMENT_SIZE, defaults.fragmentSize) ?: defaults.fragmentSize,
        )
    }

    private fun loadSavedConfigurationsInternal(preferences: SharedPreferences): MutableList<SavedTunnelConfiguration> {
        val raw = preferences.getString(KEY_SAVED_CONFIGURATIONS, null) ?: return mutableListOf()
        return runCatching {
            val jsonArray = JSONArray(raw)
            buildList {
                for (index in 0 until jsonArray.length()) {
                    val item = jsonArray.optJSONObject(index) ?: continue
                    add(SavedTunnelConfiguration.fromJsonObject(item))
                }
            }.toMutableList()
        }.getOrDefault(mutableListOf())
    }

    private fun persistSavedConfigurations(
        editor: SharedPreferences.Editor,
        configurations: List<SavedTunnelConfiguration>,
    ): SharedPreferences.Editor {
        val jsonArray = JSONArray()
        configurations.forEach { jsonArray.put(it.toJsonObject()) }
        return editor.putString(KEY_SAVED_CONFIGURATIONS, jsonArray.toString())
    }

    private fun mirrorLegacyConfiguration(
        editor: SharedPreferences.Editor,
        configuration: TunnelConfiguration,
    ): SharedPreferences.Editor {
        return editor
            .putString(KEY_SERVER_URL, configuration.serverUrl)
            .putString(KEY_SECRET, configuration.secret)
            .putString(KEY_LISTEN_PORT, configuration.listenPort)
            .putString(KEY_CDN_EDGE, configuration.cdnEdge)
            .putString(KEY_HOST_OVERRIDE, configuration.hostOverride)
            .putString(KEY_SNI_OVERRIDE, configuration.sniOverride)
            .putInt(KEY_TRANSPORT_MODE, configuration.transportMode.rawValue)
            .putBoolean(KEY_FRAGMENT_ENABLED, configuration.fragmentEnabled)
            .putString(KEY_FRAGMENT_SIZE, configuration.fragmentSize)
    }

    fun loadSavedConfigurations(context: Context): List<SavedTunnelConfiguration> {
        ensureConfigurationStore(context)
        return loadSavedConfigurationsInternal(prefs(context))
    }

    fun loadSelectedConfigurationId(context: Context): String? {
        ensureConfigurationStore(context)
        return prefs(context).getString(KEY_SELECTED_CONFIGURATION_ID, null)
    }

    fun loadSelectedConfigurationEntry(context: Context): SavedTunnelConfiguration? {
        val selectedId = loadSelectedConfigurationId(context) ?: return null
        return loadSavedConfigurations(context).firstOrNull { it.id == selectedId }
    }

    fun loadConfiguration(context: Context): TunnelConfiguration {
        return loadSelectedConfigurationEntry(context)?.configuration ?: run {
            val legacyConfiguration = loadLegacyConfiguration(prefs(context))
            if (legacyConfiguration.isEmpty) TunnelConfiguration() else legacyConfiguration
        }
    }

    fun saveConfiguration(context: Context, configuration: TunnelConfiguration) {
        ensureConfigurationStore(context)
        val preferences = prefs(context)
        val selectedEntry = loadSelectedConfigurationEntry(context)
        if (selectedEntry != null) {
            updateConfiguration(
                context = context,
                id = selectedEntry.id,
                name = selectedEntry.name,
                configuration = configuration,
            )
            return
        }

        val addedConfiguration = SavedTunnelConfiguration(configuration = configuration)
        persistSavedConfigurations(preferences.edit(), listOf(addedConfiguration))
            .putString(KEY_SELECTED_CONFIGURATION_ID, addedConfiguration.id)
            .let { mirrorLegacyConfiguration(it, configuration) }
            .commit()
    }

    fun addConfiguration(
        context: Context,
        name: String,
        configuration: TunnelConfiguration,
    ): SavedTunnelConfiguration {
        ensureConfigurationStore(context)
        val preferences = prefs(context)
        val configurations = loadSavedConfigurationsInternal(preferences)
        val savedConfiguration = SavedTunnelConfiguration(name = name, configuration = configuration)
        configurations.add(savedConfiguration)
        persistSavedConfigurations(preferences.edit(), configurations)
            .putString(KEY_SELECTED_CONFIGURATION_ID, savedConfiguration.id)
            .let { mirrorLegacyConfiguration(it, configuration) }
            .commit()
        return savedConfiguration
    }

    fun updateConfiguration(
        context: Context,
        id: String,
        name: String,
        configuration: TunnelConfiguration,
    ): SavedTunnelConfiguration? {
        ensureConfigurationStore(context)
        val preferences = prefs(context)
        val configurations = loadSavedConfigurationsInternal(preferences)
        val index = configurations.indexOfFirst { it.id == id }
        if (index == -1) {
            return null
        }

        val updatedConfiguration = configurations[index].copy(name = name, configuration = configuration)
        configurations[index] = updatedConfiguration

        val editor = persistSavedConfigurations(preferences.edit(), configurations)
        if (preferences.getString(KEY_SELECTED_CONFIGURATION_ID, null) == id) {
            mirrorLegacyConfiguration(editor, configuration)
        }
        editor.commit()
        return updatedConfiguration
    }

    fun selectConfiguration(context: Context, id: String): Boolean {
        ensureConfigurationStore(context)
        val selectedConfiguration = loadSavedConfigurations(context).firstOrNull { it.id == id } ?: return false
        mirrorLegacyConfiguration(
            prefs(context).edit().putString(KEY_SELECTED_CONFIGURATION_ID, id),
            selectedConfiguration.configuration,
        ).commit()
        return true
    }

    fun loadActiveConfigurationId(context: Context): String? {
        return prefs(context).getString(KEY_ACTIVE_CONFIGURATION_ID, null)
    }

    fun loadActiveConfiguration(context: Context): TunnelConfiguration? {
        val raw = prefs(context).getString(KEY_ACTIVE_CONFIGURATION_JSON, null) ?: return null
        return runCatching {
            TunnelConfiguration.fromJsonObject(JSONObject(raw))
        }.getOrNull()
    }

    fun loadActiveConfigurationDisplayName(context: Context): String? {
        val activeId = loadActiveConfigurationId(context)
        if (activeId != null) {
            loadSavedConfigurations(context).firstOrNull { it.id == activeId }?.let { return it.displayName }
        }

        return loadActiveConfiguration(context)?.suggestedName
    }

    fun setActiveConfiguration(
        context: Context,
        id: String?,
        configuration: TunnelConfiguration?,
    ) {
        val editor = prefs(context).edit()
        if (id != null) {
            editor.putString(KEY_ACTIVE_CONFIGURATION_ID, id)
        } else {
            editor.remove(KEY_ACTIVE_CONFIGURATION_ID)
        }

        if (configuration != null) {
            editor.putString(KEY_ACTIVE_CONFIGURATION_JSON, configuration.toJsonObject().toString())
        } else {
            editor.remove(KEY_ACTIVE_CONFIGURATION_JSON)
        }
        editor.commit()
    }

    fun markSelectedConfigurationActive(context: Context, configuration: TunnelConfiguration = loadConfiguration(context)) {
        setActiveConfiguration(context, loadSelectedConfigurationId(context), configuration)
    }

    fun isVpnDisclosureAcknowledged(context: Context): Boolean {
        return prefs(context).getBoolean(KEY_VPN_DISCLOSURE_ACKNOWLEDGED, false)
    }

    fun setVpnDisclosureAcknowledged(context: Context, acknowledged: Boolean) {
        prefs(context).edit()
            .putBoolean(KEY_VPN_DISCLOSURE_ACKNOWLEDGED, acknowledged)
            .apply()
    }

    fun loadSnapshot(context: Context): TunnelSnapshot {
        val prefs = prefs(context)
        val rawState = prefs.getString(KEY_STATE, TunnelState.IDLE.name) ?: TunnelState.IDLE.name
        val state = runCatching { TunnelState.valueOf(rawState) }.getOrDefault(TunnelState.IDLE)
        val message = prefs.getString(KEY_MESSAGE, "Ready") ?: "Ready"
        val updatedAtMs = prefs.getLong(KEY_STATE_UPDATED_AT, 0L)
        return TunnelSnapshot(state = state, message = message, updatedAtMs = updatedAtMs)
    }

    fun updateState(context: Context, state: TunnelState, message: String) {
        val updatedAtMs = System.currentTimeMillis()
        prefs(context)
            .edit()
            .putString(KEY_STATE, state.name)
            .putString(KEY_MESSAGE, message)
            .putLong(KEY_STATE_UPDATED_AT, updatedAtMs)
            .commit()
        TunnelEvents.broadcastState(context, state.name, message)
    }

    fun loadRuntimeSnapshot(context: Context): TunnelRuntimeSnapshot {
        val raw = prefs(context).getString(KEY_RUNTIME_JSON, null)
        return TunnelRuntimeSnapshot.fromJsonString(raw)
    }

    fun loadDiagnostics(context: Context): TunnelDiagnosticsSnapshot {
        val raw = prefs(context).getString(KEY_DIAGNOSTICS_JSON, null)
        return TunnelDiagnosticsSnapshot.fromJsonString(raw)
    }

    fun updateRuntimeSnapshot(context: Context, snapshot: TunnelRuntimeSnapshot) {
        val json = snapshot.toJsonString()
        prefs(context)
            .edit()
            .putString(KEY_RUNTIME_JSON, json)
            .commit()
        val diagJson = prefs(context).getString(KEY_DIAGNOSTICS_JSON, null)
        TunnelEvents.broadcastDashboard(context, json, diagJson)
    }

    fun updateDiagnostics(context: Context, diagnostics: TunnelDiagnosticsSnapshot) {
        val json = diagnostics.toJsonString()
        prefs(context)
            .edit()
            .putString(KEY_DIAGNOSTICS_JSON, json)
            .commit()
        val runtimeJson = prefs(context).getString(KEY_RUNTIME_JSON, null)
        TunnelEvents.broadcastDashboard(context, runtimeJson, json)
    }

    fun syncStateLocally(context: Context, stateName: String, message: String) {
        prefs(context).edit()
            .putString(KEY_STATE, stateName)
            .putString(KEY_MESSAGE, message)
            .apply()
    }

    fun syncDashboardLocally(context: Context, runtimeJson: String?, diagJson: String?) {
        val editor = prefs(context).edit()
        if (runtimeJson != null) editor.putString(KEY_RUNTIME_JSON, runtimeJson)
        if (diagJson != null) editor.putString(KEY_DIAGNOSTICS_JSON, diagJson)
        editor.apply()
    }
}
