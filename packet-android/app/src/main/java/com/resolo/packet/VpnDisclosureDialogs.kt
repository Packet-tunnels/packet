package com.resolo.packet

import android.app.Activity
import android.app.AlertDialog

object VpnDisclosureDialogs {
    private const val DISCLOSURE_TITLE = "VPN Data-Use Disclosure"
    private const val DISCLOSURE_MESSAGE =
        "Packet creates an Android VPN connection and routes your device traffic through the configured tunnel while connected.\n\n" +
            "Your server settings and disclosure acknowledgement are stored locally on this device so the tunnel can reconnect with the same configuration.\n\n" +
            "You can disconnect at any time from Packet or from Android VPN settings."

    fun show(
        activity: Activity,
        acceptTitle: String,
        dismissTitle: String,
        onAccept: () -> Unit,
        onDismiss: (() -> Unit)? = null,
    ) {
        AlertDialog.Builder(activity)
            .setTitle(DISCLOSURE_TITLE)
            .setMessage(DISCLOSURE_MESSAGE)
            .setPositiveButton(acceptTitle) { dialog, _ ->
                TunnelPreferences.setVpnDisclosureAcknowledged(activity, true)
                dialog.dismiss()
                onAccept()
            }
            .setNegativeButton(dismissTitle) { dialog, _ ->
                dialog.dismiss()
                onDismiss?.invoke()
            }
            .show()
    }
}
