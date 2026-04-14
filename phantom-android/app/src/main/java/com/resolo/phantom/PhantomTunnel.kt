package com.resolo.phantom

class PhantomTunnel {
    interface LogCallback {
        fun onLog(message: String)
    }

    companion object {
        init {
            System.loadLibrary("phantom_client")
        }

        @JvmStatic
        external fun setLogCallback(callback: LogCallback)

        @JvmStatic
        external fun emitTestOutput()

        @JvmStatic
        external fun startClient(serverUrl: String, secret: String, listenPort: Int)

        @JvmStatic
        external fun startClientCdn(
            serverUrl: String,
            secret: String,
            listenPort: Int,
            cdnEdge: String,
            hostOverride: String,
            transportMode: Int
        )
    }
}
