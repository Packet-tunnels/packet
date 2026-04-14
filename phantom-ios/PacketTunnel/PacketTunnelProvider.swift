import Darwin
import Foundation
import NetworkExtension

final class PacketTunnelProvider: NEPacketTunnelProvider {
    override init() {
        super.init()
        TunnelRuntimeBridge.installLogCallback()
    }

    override func startTunnel(
        options: [String: NSObject]?,
        completionHandler: @escaping (Error?) -> Void
    ) {
        SharedTunnelLogStore.append("[EXT] Tunnel start requested")

        Task {
            do {
                try await startProviderTunnel()
                completionHandler(nil)
            } catch {
                SharedTunnelLogStore.append("[EXT] Failed to start tunnel: \(error.localizedDescription)")
                completionHandler(error)
            }
        }
    }

    override func stopTunnel(
        with reason: NEProviderStopReason,
        completionHandler: @escaping () -> Void
    ) {
        SharedTunnelLogStore.append("[EXT] Tunnel stop requested (\(reason.rawValue))")
        completionHandler()
    }

    override func handleAppMessage(_ messageData: Data, completionHandler: ((Data?) -> Void)? = nil) {
        let command = String(data: messageData, encoding: .utf8)

        switch command {
        case "stats":
            completionHandler?(TunnelRuntimeBridge.runtimeStatsData())
        default:
            completionHandler?(nil)
        }
    }

    private func startProviderTunnel() async throws {
        let configuration = try loadConfiguration()
        let port = try requireListenPort(from: configuration)

        SharedTunnelLogStore.append("[EXT] Starting Rust tunnel core on 127.0.0.1:\(port)")

        let result = TunnelRuntimeBridge.startRustClient(with: configuration)
        guard result == 0 else {
            throw PacketTunnelError.rustStartFailed(result)
        }

        try await waitForLocalProxy(port: port)
        SharedTunnelLogStore.append("[EXT] Applying proxy-routed tunnel settings")
        try await applyNetworkSettings(makeNetworkSettings(for: configuration, port: port))

        SharedTunnelLogStore.append("[EXT] Packet tunnel settings applied")
    }

    private func loadConfiguration() throws -> TunnelConfiguration {
        guard
            let tunnelProtocol = protocolConfiguration as? NETunnelProviderProtocol,
            let providerConfiguration = tunnelProtocol.providerConfiguration
        else {
            throw PacketTunnelError.missingConfiguration
        }

        let configuration = TunnelConfiguration(providerConfiguration: providerConfiguration)

        guard !configuration.normalizedServerURL.isEmpty else {
            throw PacketTunnelError.invalidConfiguration("Server URL is required.")
        }

        guard !configuration.normalizedSecret.isEmpty else {
            throw PacketTunnelError.invalidConfiguration("Shared secret is required.")
        }

        return configuration
    }

    private func requireListenPort(from configuration: TunnelConfiguration) throws -> UInt16 {
        guard let port = configuration.listenPortValue, port > 0 else {
            throw PacketTunnelError.invalidConfiguration("Listen port must be between 1 and 65535.")
        }

        return port
    }

    private func makeNetworkSettings(
        for configuration: TunnelConfiguration,
        port: UInt16
    ) -> NEPacketTunnelNetworkSettings {
        let settings = NEPacketTunnelNetworkSettings(tunnelRemoteAddress: configuration.remoteAddress)
        settings.mtu = 1500

        // The Rust core currently exposes a local SOCKS proxy. Do not publish
        // default TUN routes here because packetFlow is not bridged yet.
        let ipv4Settings = NEIPv4Settings(addresses: ["10.8.0.2"], subnetMasks: ["255.255.255.255"])
        ipv4Settings.includedRoutes = []
        settings.ipv4Settings = ipv4Settings

        let ipv6Settings = NEIPv6Settings(
            addresses: ["fd84:306d:fc4e::2"],
            networkPrefixLengths: [64]
        )
        ipv6Settings.includedRoutes = []
        settings.ipv6Settings = ipv6Settings

        let proxySettings = NEProxySettings()
        proxySettings.autoProxyConfigurationEnabled = true
        proxySettings.proxyAutoConfigurationJavaScript = makeProxyAutoConfigurationScript(port: port)
        proxySettings.excludeSimpleHostnames = true
        proxySettings.matchDomains = [""]
        proxySettings.exceptionList = configuration.proxyExceptionHosts
        settings.proxySettings = proxySettings

        return settings
    }

    private func applyNetworkSettings(_ settings: NEPacketTunnelNetworkSettings) async throws {
        try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
            setTunnelNetworkSettings(settings) { error in
                if let error {
                    continuation.resume(throwing: error)
                    return
                }

                continuation.resume(returning: ())
            }
        }
    }

    private func waitForLocalProxy(port: UInt16) async throws {
        let timeoutAt = Date().addingTimeInterval(5)

        while Date() < timeoutAt {
            if canConnectToLocalProxy(port: port) {
                SharedTunnelLogStore.append("[EXT] Local SOCKS5 listener is ready")
                return
            }

            try await Task.sleep(nanoseconds: 200_000_000)
        }

        throw PacketTunnelError.proxyNotReady
    }

    private func makeProxyAutoConfigurationScript(port: UInt16) -> String {
        """
        function FindProxyForURL(url, host) {
            return "SOCKS5 127.0.0.1:\(port); SOCKS 127.0.0.1:\(port); DIRECT";
        }
        """
    }

    private func canConnectToLocalProxy(port: UInt16) -> Bool {
        let socketDescriptor = socket(AF_INET, SOCK_STREAM, 0)
        guard socketDescriptor >= 0 else { return false }
        defer { close(socketDescriptor) }

        var address = sockaddr_in()
        address.sin_len = UInt8(MemoryLayout<sockaddr_in>.stride)
        address.sin_family = sa_family_t(AF_INET)
        address.sin_port = port.bigEndian

        let conversionResult = "127.0.0.1".withCString { pointer in
            inet_pton(AF_INET, pointer, &address.sin_addr)
        }
        guard conversionResult == 1 else { return false }

        let connectResult = withUnsafePointer(to: &address) { pointer in
            pointer.withMemoryRebound(to: sockaddr.self, capacity: 1) { sockaddrPointer in
                connect(
                    socketDescriptor,
                    sockaddrPointer,
                    socklen_t(MemoryLayout<sockaddr_in>.stride)
                )
            }
        }

        return connectResult == 0
    }
}

private enum PacketTunnelError: LocalizedError {
    case missingConfiguration
    case invalidConfiguration(String)
    case rustStartFailed(Int32)
    case proxyNotReady

    var errorDescription: String? {
        switch self {
        case .missingConfiguration:
            return "Missing NetworkExtension provider configuration."
        case let .invalidConfiguration(message):
            return message
        case let .rustStartFailed(code):
            return "Rust tunnel core failed to start with code \(code)."
        case .proxyNotReady:
            return "The local SOCKS5 listener did not become ready in time."
        }
    }
}
