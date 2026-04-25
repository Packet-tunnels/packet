package com.resolo.phantom

import com.resolo.packet.PacketBridge

object PhantomTunnel {
    init {
        System.loadLibrary("phantom_client")
    }

    @JvmStatic
    external fun setLogCallback(callback: PacketBridge.LogCallback)

    @JvmStatic
    external fun emitTestOutput()

    @JvmStatic
    external fun copyStatsJson(): String?

    @JvmStatic
    external fun copyMeshStatsJson(): String?

    @JvmStatic
    external fun stopClient()

    @JvmStatic
    external fun startClient(serverUrl: String, secret: String, listenPort: Int): Int

    @JvmStatic
    external fun startMeshClient(configJson: String, listenPort: Int): Int

    @JvmStatic
    external fun importMeshPeers(peersJson: String): Int

    @JvmStatic
    external fun startClientCdn(
        serverUrl: String,
        secret: String,
        listenPort: Int,
        cdnEdge: String,
        hostOverride: String,
        transportMode: Int,
    ): Int

    @JvmStatic
    external fun startClientFull(
        serverUrl: String,
        secret: String,
        listenPort: Int,
        cdnEdge: String,
        hostOverride: String,
        sniOverride: String,
        transportMode: Int,
        fragmentEnabled: Boolean,
        fragmentSize: Int,
    ): Int

    @JvmStatic
    external fun startTun2Socks(
        tunFd: Int,
        socksAddress: String,
        socksPort: Int,
        mtu: Int,
        dnsAddress: String,
    ): Int

    @JvmStatic
    external fun stopTun2Socks()
}
