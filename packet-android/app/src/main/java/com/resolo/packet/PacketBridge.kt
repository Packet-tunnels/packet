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
        fun copyMeshStatsJson(): String? = PhantomTunnel.copyMeshStatsJson()

        @JvmStatic
        fun stopClient() {
            PhantomTunnel.stopClient()
        }

        @JvmStatic
        fun startLayeredCarrier(trojanUri: String, listenPort: Int): Int =
            PhantomTunnel.startLayeredCarrier(trojanUri, listenPort)

        @JvmStatic
        fun startLayeredCarrierFull(
            trojanUri: String,
            listenPort: Int,
            fragmentEnabled: Boolean,
            fragmentSize: Int,
        ): Int =
            PhantomTunnel.startLayeredCarrierFull(
                trojanUri,
                listenPort,
                fragmentEnabled,
                fragmentSize,
            )

        @JvmStatic
        fun stopLayeredCarrier() {
            PhantomTunnel.stopLayeredCarrier()
        }

        @JvmStatic
        fun startClient(serverUrl: String, secret: String, listenPort: Int): Int =
            PhantomTunnel.startClient(serverUrl, secret, listenPort)

        @JvmStatic
        fun startMeshClient(configJson: String, listenPort: Int): Int =
            PhantomTunnel.startMeshClient(configJson, listenPort)

        @JvmStatic
        fun importMeshPeers(peersJson: String): Int =
            PhantomTunnel.importMeshPeers(peersJson)

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
            transportMode: Int,
            fragmentEnabled: Boolean,
            fragmentSize: Int,
            obfsKey: String,
            upstreamProxy: String,
        ): Int =
            PhantomTunnel.startClientFull(
                serverUrl,
                secret,
                listenPort,
                cdnEdge,
                hostOverride,
                sniOverride,
                transportMode,
                fragmentEnabled,
                fragmentSize,
                obfsKey,
                upstreamProxy,
            )

        @JvmStatic
        fun startClientPrivateRelay(serverUrl: String, secret: String, listenPort: Int): Int =
            PhantomTunnel.startClientPrivateRelay(serverUrl, secret, listenPort)

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
