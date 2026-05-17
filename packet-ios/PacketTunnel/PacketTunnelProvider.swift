import Darwin
import Foundation
import NetworkExtension

final class PacketTunnelProvider: NEPacketTunnelProvider {
    private var listenPort: UInt16?

    override init() {
        super.init()
        TunnelRuntimeBridge.installLogCallback()
    }

    override func startTunnel(
        options: [String: NSObject]? = nil,
        completionHandler: @escaping @Sendable (Error?) -> Void
    ) {
        SharedTunnelLogStore.append("[EXT] Packet tunnel start requested")
        NSLog("[PHANTOM] EXT: startTunnel requested")

        Task {
            do {
                NSLog("[PHANTOM] EXT: Starting tunnel task...")
                try await startProxyTunnel()
                NSLog("[PHANTOM] EXT: Proxy tunnel started, calling completionHandler")
                completionHandler(nil)
            } catch {
                NSLog("[PHANTOM] EXT: Failed to start tunnel: \(error.localizedDescription)")
                SharedTunnelLogStore.append("[EXT] Failed to start packet tunnel: \(error.localizedDescription)")
                completionHandler(error)
            }
        }
    }

    override func stopTunnel(
        with reason: NEProviderStopReason,
        completionHandler: @escaping () -> Void
    ) {
        listenPort = nil
        SharedTunnelLogStore.append("[EXT] Packet tunnel stop requested (\(reason.rawValue))")
        completionHandler()
        // Kill the extension process to release port 1080 held by Rust threads.
        // iOS will spawn a fresh process on the next connect.
        exit(0)
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

    private func startProxyTunnel() async throws {
        NSLog("[PHANTOM] EXT: startProxyTunnel() loading configuration")
        SharedTunnelLogStore.append("[EXT] Loading tunnel configuration...")
        let configuration = try loadConfiguration()
        let requestedPort = try requestedListenPort(from: configuration)
        let requestedPortLabel = requestedPort == 0 ? "auto" : "\(requestedPort)"

        SharedTunnelLogStore.append("[EXT] Starting Rust tunnel core on 127.0.0.1:\(requestedPortLabel)")
        NSLog("[PHANTOM] EXT: Calling startRustClient for port request \(requestedPortLabel)")

        var result: Int32 = -99
        var boundPort: UInt16?
        let maxRetries = 5

        for attempt in 1...maxRetries {
            result = TunnelRuntimeBridge.startRustClient(with: configuration)
            NSLog("[PHANTOM] EXT: startRustClient attempt \(attempt)/\(maxRetries) returned \(result)")
            SharedTunnelLogStore.append("[EXT] Rust start attempt \(attempt)/\(maxRetries): code \(result)")

            if result > 0, let actualPort = UInt16(exactly: result) {
                boundPort = actualPort
                break // Success
            }

            if result == -2, attempt < maxRetries {
                // Port still held by previous extension process — wait for OS to reclaim it
                SharedTunnelLogStore.append("[EXT] Port in use, retrying in 1s...")
                try await Task.sleep(nanoseconds: 1_000_000_000) // 1 second
                continue
            }

            // Non-retryable error or final attempt failed
            SharedTunnelLogStore.append("[EXT] ❌ Rust failed to start (code \(result))")
            throw PacketTunnelError.rustStartFailed(result)
        }

        guard let port = boundPort else {
            throw PacketTunnelError.rustStartFailed(result)
        }

        listenPort = port

        if requestedPort == 0 {
            SharedTunnelLogStore.append("[EXT] Auto-selected local SOCKS5 port \(port)")
        } else if requestedPort != port {
            SharedTunnelLogStore.append(
                "[EXT] Requested local SOCKS5 port \(requestedPort) was busy, using \(port) instead"
            )
        } else {
            SharedTunnelLogStore.append("[EXT] Using requested local SOCKS5 port \(port)")
        }

        NSLog("[PHANTOM] EXT: Waiting for local proxy on port \(port)...")
        SharedTunnelLogStore.append("[EXT] Waiting for SOCKS5 to be ready on port \(port)...")
        try await waitForLocalProxy(port: port)
        NSLog("[PHANTOM] EXT: Local proxy is ready!")
        SharedTunnelLogStore.append("[EXT] SOCKS5 proxy is ready on 127.0.0.1:\(port)")

        NSLog("[PHANTOM] EXT: Setting up proxy-routed tunnel settings")
        SharedTunnelLogStore.append("[EXT] Applying proxy-routed tunnel settings...")
        try await applyNetworkSettings(makeNetworkSettings(for: configuration, port: port))
        NSLog("[PHANTOM] EXT: Tunnel network settings applied successfully")
        SharedTunnelLogStore.append("[EXT] Tunnel is fully configured and active")
    }

    private func loadConfiguration() throws -> TunnelConfiguration {
        guard
            let tunnelProtocol = protocolConfiguration as? NETunnelProviderProtocol,
            let providerConfiguration = tunnelProtocol.providerConfiguration
        else {
            NSLog("[PHANTOM] EXT: ❌ MISSING protocolConfiguration or providerConfiguration")
            SharedTunnelLogStore.append("[EXT] ❌ Missing tunnel protocol configuration")
            throw PacketTunnelError.missingConfiguration
        }

        let configuration = TunnelConfiguration(providerConfiguration: providerConfiguration)

        NSLog("[PHANTOM] EXT: Config - URL='\(configuration.normalizedServerURL)' secretLen=\(configuration.normalizedSecret.count) port='\(configuration.listenPort)'")
        SharedTunnelLogStore.append("[EXT] Config loaded: URL='\(configuration.normalizedServerURL)' port=\(configuration.listenPort)")

        guard !configuration.normalizedServerURL.isEmpty else {
            NSLog("[PHANTOM] EXT: ❌ Server URL is empty")
            SharedTunnelLogStore.append("[EXT] ❌ Server URL is empty")
            throw PacketTunnelError.invalidConfiguration("Server URL is required.")
        }

        guard !configuration.normalizedSecret.isEmpty else {
            NSLog("[PHANTOM] EXT: ❌ Secret is empty")
            SharedTunnelLogStore.append("[EXT] ❌ Secret is empty")
            throw PacketTunnelError.invalidConfiguration("Shared secret is required.")
        }

        if let advancedValidationError = configuration.advancedValidationError {
            NSLog("[PHANTOM] EXT: ❌ Invalid advanced tunnel configuration")
            SharedTunnelLogStore.append("[EXT] ❌ \(advancedValidationError)")
            throw PacketTunnelError.invalidConfiguration(advancedValidationError)
        }

        return configuration
    }

    private func requestedListenPort(from configuration: TunnelConfiguration) throws -> UInt16 {
        NSLog("[PHANTOM] EXT: Listen port string='\(configuration.listenPort)' parsed=\(String(describing: configuration.listenPortValue))")

        let trimmedPort = configuration.listenPort.trimmingCharacters(in: .whitespacesAndNewlines)
        if trimmedPort.isEmpty || trimmedPort.lowercased() == "auto" {
            return 0
        }

        guard let port = configuration.listenPortValue, port >= 1024 else {
            NSLog("[PHANTOM] EXT: ❌ Invalid listen port '\(configuration.listenPort)'")
            SharedTunnelLogStore.append("[EXT] ❌ Invalid listen port '\(configuration.listenPort)' — must be 1024-65535")
            throw PacketTunnelError.invalidConfiguration("Local SOCKS port must be 1024-65535, or leave it blank for auto.")
        }

        return port
    }

    private func makeNetworkSettings(
        for configuration: TunnelConfiguration,
        port: UInt16
    ) -> NEPacketTunnelNetworkSettings {
        let tunnelRemoteAddress = validatedTunnelRemoteAddress(for: configuration)
        SharedTunnelLogStore.append("[EXT] Using tunnel remote address \(tunnelRemoteAddress)")

        let settings = NEPacketTunnelNetworkSettings(tunnelRemoteAddress: tunnelRemoteAddress)
        settings.mtu = 1500

        // The Rust core currently exposes a local SOCKS proxy only. Do not
        // advertise a full-device default route until packetFlow is bridged.
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
        proxySettings.exceptionList = configuration.proxyExceptionHosts
        proxySettings.matchDomains = [""]
        settings.proxySettings = proxySettings

        return settings
    }

    private func validatedTunnelRemoteAddress(for configuration: TunnelConfiguration) -> String {
        let candidates = [configuration.endpointHost, configuration.remoteAddress]

        for candidate in candidates {
            let trimmed = candidate.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !trimmed.isEmpty else { continue }

            if isIPAddress(trimmed) {
                return trimmed
            }
        }

        // Packet tunnel settings require an IP literal. For proxy-only mode,
        // loopback is a safe fallback when the configured endpoint is a hostname.
        return "127.0.0.1"
    }

    private func isIPAddress(_ value: String) -> Bool {
        var ipv4 = in_addr()
        var ipv6 = in6_addr()

        let isIPv4 = value.withCString { pointer in
            inet_pton(AF_INET, pointer, &ipv4)
        } == 1

        if isIPv4 {
            return true
        }

        return value.withCString { pointer in
            inet_pton(AF_INET6, pointer, &ipv6)
        } == 1
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
        // Give Rust 10 seconds to bind and start listening
        let timeoutAt = Date().addingTimeInterval(10)
        var attempts = 0

        while Date() < timeoutAt {
            attempts += 1
            if canConnectToLocalProxy(port: port) {
                SharedTunnelLogStore.append("[EXT] Local SOCKS5 listener is ready (attempt \(attempts))")
                return
            }
            if attempts % 5 == 0 {
                SharedTunnelLogStore.append("[EXT] Still waiting for SOCKS5... (\(attempts) attempts)")
            }
            try await Task.sleep(nanoseconds: 200_000_000)
        }

        SharedTunnelLogStore.append("[EXT] ❌ Timed out waiting for SOCKS5 on port \(port) after 10s")
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
