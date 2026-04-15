package com.resolo.packet

import com.resolo.phantom.PhantomTunnel

class PacketBridge {
    interface LogCallback {
        fun onLog(message: String)
    }

    companion object {
        @JvmStatic
        fun setLogCallback(callback: LogCallback) {
            PhantomTunnel.setLogCallback(callback)
        }

        @JvmStatic
        fun emitTestOutput() {
            PhantomTunnel.emitTestOutput()
        }

        @JvmStatic
        fun copyStatsJson(): String? = PhantomTunnel.copyStatsJson()

        @JvmStatic
        fun startClient(serverUrl: String, secret: String, listenPort: Int): Int =
            PhantomTunnel.startClient(serverUrl, secret, listenPort)

        @JvmStatic
        fun startClientCdn(
            serverUrl: String,
            secret: String,
            listenPort: Int,
            cdnEdge: String,
            hostOverride: String,
            transportMode: Int
        ): Int =
            PhantomTunnel.startClientCdn(
                serverUrl,
                secret,
                listenPort,
                cdnEdge,
                hostOverride,
                transportMode,
            )

        @JvmStatic
        fun startClientFull(
            serverUrl: String,
            secret: String,
            listenPort: Int,
            cdnEdge: String,
            hostOverride: String,
            sniOverride: String,
            transportMode: Int
        ): Int =
            PhantomTunnel.startClientFull(
                serverUrl,
                secret,
                listenPort,
                cdnEdge,
                hostOverride,
                sniOverride,
                transportMode,
            )

        @JvmStatic
        fun startTun2Socks(
            tunFd: Int,
            socksAddress: String,
            socksPort: Int,
            mtu: Int,
            dnsAddress: String
        ): Int =
            PhantomTunnel.startTun2Socks(tunFd, socksAddress, socksPort, mtu, dnsAddress)

        @JvmStatic
        fun stopTun2Socks() {
            PhantomTunnel.stopTun2Socks()
        }
    }
}
