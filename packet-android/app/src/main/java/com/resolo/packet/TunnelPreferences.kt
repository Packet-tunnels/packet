package com.resolo.packet

import android.content.Context
import android.content.Intent

object TunnelEvents {
    fun broadcastState(context: Context, stateName: String, message: String) {
        val intent = Intent(TunnelActions.ACTION_STATE_UPDATED)
            .setPackage(context.packageName)
            .putExtra("state", stateName)
            .putExtra("message", message)
        context.sendBroadcast(intent)
    }

    fun broadcastLog(context: Context) {
        context.sendBroadcast(Intent(TunnelActions.ACTION_LOG_UPDATED).setPackage(context.packageName))
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

    private const val KEY_SERVER_URL = "server_url"
    private const val KEY_SECRET = "secret"
    private const val KEY_LISTEN_PORT = "listen_port"
    private const val KEY_CDN_EDGE = "cdn_edge"
    private const val KEY_HOST_OVERRIDE = "host_override"
    private const val KEY_SNI_OVERRIDE = "sni_override"
    private const val KEY_TRANSPORT_MODE = "transport_mode"
    private const val KEY_STATE = "state"
    private const val KEY_MESSAGE = "message"
    private const val KEY_STATE_UPDATED_AT = "state_updated_at"
    private const val KEY_RUNTIME_JSON = "runtime_json"
    private const val KEY_DIAGNOSTICS_JSON = "diagnostics_json"

    private fun prefs(context: Context) =
        context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)

    fun loadConfiguration(context: Context): TunnelConfiguration {
        val defaults = TunnelConfiguration()
        val prefs = prefs(context)
        return TunnelConfiguration(
            serverUrl = prefs.getString(KEY_SERVER_URL, defaults.serverUrl) ?: defaults.serverUrl,
            secret = prefs.getString(KEY_SECRET, defaults.secret) ?: defaults.secret,
            listenPort = prefs.getString(KEY_LISTEN_PORT, defaults.listenPort) ?: defaults.listenPort,
            cdnEdge = prefs.getString(KEY_CDN_EDGE, defaults.cdnEdge) ?: defaults.cdnEdge,
            hostOverride = prefs.getString(KEY_HOST_OVERRIDE, defaults.hostOverride) ?: defaults.hostOverride,
            sniOverride = prefs.getString(KEY_SNI_OVERRIDE, defaults.sniOverride) ?: defaults.sniOverride,
            transportMode = TunnelTransportMode.fromRawValue(
                prefs.getInt(KEY_TRANSPORT_MODE, defaults.transportMode.rawValue)
            ),
        )
    }

    fun saveConfiguration(context: Context, configuration: TunnelConfiguration) {
        prefs(context)
            .edit()
            .putString(KEY_SERVER_URL, configuration.serverUrl)
            .putString(KEY_SECRET, configuration.secret)
            .putString(KEY_LISTEN_PORT, configuration.listenPort)
            .putString(KEY_CDN_EDGE, configuration.cdnEdge)
            .putString(KEY_HOST_OVERRIDE, configuration.hostOverride)
            .putString(KEY_SNI_OVERRIDE, configuration.sniOverride)
            .putInt(KEY_TRANSPORT_MODE, configuration.transportMode.rawValue)
            .commit()
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
