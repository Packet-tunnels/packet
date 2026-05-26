package com.resolo.packet

import android.content.Context
import android.net.ConnectivityManager
import java.net.URLEncoder

/**
 * Some networks require traffic to route through a system-configured HTTP proxy.
 * This is especially common on restricted cellular networks.
 *
 * This helper detects if Android has a default system proxy configured and
 * rewrites the carrier URI to chain through it via `upstream_http=...`.
 * An operator-supplied upstream is never overwritten.
 *
 * Works for both cellular APN proxies and Wi-Fi manual proxy settings.
 */
object SystemProxyDetector {

    /** Returns "host:port" if Android has a default proxy, else null. */
    fun detect(ctx: Context): String? {
        val cm = ctx.getSystemService(Context.CONNECTIVITY_SERVICE) as? ConnectivityManager
        val proxy = cm?.defaultProxy
        if (proxy != null && !proxy.host.isNullOrBlank() && proxy.port > 0) {
            return "${proxy.host}:${proxy.port}"
        }
        // Some carriers expose the proxy only via JVM system properties.
        val host = System.getProperty("http.proxyHost")
        val port = System.getProperty("http.proxyPort")?.toIntOrNull()
        if (!host.isNullOrBlank() && port != null && port > 0) {
            return "$host:$port"
        }
        return null
    }

    /**
     * Append `upstream_http=<hostPort>` to a `trojan://...` URI, before the
     * fragment, unless the URI already specifies an upstream.
     */
    fun appendToTrojanUri(uri: String, hostPort: String): String {
        if (UPSTREAM_RX.containsMatchIn(uri)) return uri
        val hashIdx = uri.indexOf('#')
        val base = if (hashIdx >= 0) uri.substring(0, hashIdx) else uri
        val fragment = if (hashIdx >= 0) uri.substring(hashIdx) else ""
        val sep = if (base.contains('?')) '&' else '?'
        val encoded = URLEncoder.encode(hostPort, "UTF-8")
        return "${base}${sep}upstream_http=${encoded}${fragment}"
    }

    private val UPSTREAM_RX = Regex("[?&]upstream(_proxy|_http|_socks|_socks5)?=")
}
