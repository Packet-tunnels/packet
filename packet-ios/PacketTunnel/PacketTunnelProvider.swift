import Darwin
import Foundation
import NetworkExtension

final class PacketTunnelProvider: NEPacketTunnelProvider {
    private var listenPort: UInt16?
    private var egressMetadata = TunnelEgressMetadata.empty
    private let egressProbeTargets: [EgressProbeTarget] = [
        EgressProbeTarget(host: "cloudflare.com", path: "/cdn-cgi/trace"),
        EgressProbeTarget(host: "ip-api.com", path: "/line/?fields=country,countryCode,query"),
        EgressProbeTarget(host: "connectivitycheck.gstatic.com", path: "/generate_204"),
        EgressProbeTarget(host: "example.com", path: "/"),
        EgressProbeTarget(host: "neverssl.com", path: "/")
    ]

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
                PacketPsiphonCore.shared.stop()
                TunnelRuntimeBridge.stopRustClient()
                completionHandler(error)
            }
        }
    }

    override func stopTunnel(
        with reason: NEProviderStopReason,
        completionHandler: @escaping () -> Void
    ) {
        listenPort = nil
        egressMetadata = .empty
        PacketPsiphonCore.shared.stop()
        TunnelRuntimeBridge.stopRustClient()
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
            completionHandler?(runtimeStatsDataWithEgressMetadata())
        default:
            completionHandler?(nil)
        }
    }

    private func runtimeStatsDataWithEgressMetadata() -> Data? {
        guard let statsData = TunnelRuntimeBridge.runtimeStatsData() else {
            return nil
        }

        guard let jsonObject = try? JSONSerialization.jsonObject(with: statsData),
            var object = jsonObject as? [String: Any]
        else {
            return statsData
        }

        if let pingMs = egressMetadata.pingMs {
            object["egress_ping_ms"] = pingMs
            if object["last_ping_ms"] == nil || object["last_ping_ms"] is NSNull {
                object["last_ping_ms"] = pingMs
            }
        }

        if let target = egressMetadata.target?.nilIfEmpty {
            object["egress_target"] = target
        }

        if let countryCode = egressMetadata.countryCode?.nilIfEmpty {
            object["server_country_code"] = countryCode
        }

        if let countryName = egressMetadata.countryName?.nilIfEmpty {
            object["server_country_name"] = countryName
        }

        return try? JSONSerialization.data(withJSONObject: object)
    }

    private func startProxyTunnel() async throws {
        egressMetadata = .empty
        NSLog("[PHANTOM] EXT: startProxyTunnel() loading configuration")
        SharedTunnelLogStore.append("[EXT] Loading tunnel configuration...")
        let configuration = try loadConfiguration()
        let requestedPort = try requestedListenPort(from: configuration)
        let requestedPortLabel = requestedPort == 0 ? "auto" : "\(requestedPort)"
        let localProxyLabel = configuration.usesCustomCarrier
            ? "DirectSock proxy"
            : configuration.usesPsiphonChain ? "Packet-over-Psiphon SOCKS5" : "SOCKS5"

        SharedTunnelLogStore.append("[EXT] Starting Rust tunnel core on 127.0.0.1:\(requestedPortLabel)")
        NSLog("[PHANTOM] EXT: Calling startRustClient for port request \(requestedPortLabel)")

        var result: Int32 = -99
        var boundPort: UInt16?
        let maxRetries = 5

        for attempt in 1...maxRetries {
            result = try await startRuntime(for: configuration)
            NSLog("[PHANTOM] EXT: startRustClient attempt \(attempt)/\(maxRetries) returned \(result)")
            SharedTunnelLogStore.append("[EXT] Rust start attempt \(attempt)/\(maxRetries): code \(result)")

            if result > 0, let actualPort = UInt16(exactly: result) {
                boundPort = actualPort
                break // Success
            }

            if result == -2, attempt < maxRetries {
                // Port still held by previous extension process — wait for OS to reclaim it
                SharedTunnelLogStore.append("[EXT] Port in use, retrying in 1s...")
                PacketPsiphonCore.shared.stop()
                TunnelRuntimeBridge.stopRustClient()
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
            SharedTunnelLogStore.append("[EXT] Auto-selected local \(localProxyLabel) port \(port)")
        } else if requestedPort != port {
            SharedTunnelLogStore.append(
                "[EXT] Requested local \(localProxyLabel) port \(requestedPort) was busy, using \(port) instead"
            )
        } else {
            SharedTunnelLogStore.append("[EXT] Using requested local \(localProxyLabel) port \(port)")
        }

        NSLog("[PHANTOM] EXT: Waiting for local proxy on port \(port)...")
        SharedTunnelLogStore.append("[EXT] Waiting for \(localProxyLabel) to be ready on port \(port)...")
        try await waitForLocalProxy(port: port)
        NSLog("[PHANTOM] EXT: Local proxy is ready!")
        SharedTunnelLogStore.append("[EXT] \(localProxyLabel) is ready on 127.0.0.1:\(port)")

        let egressProbe = try await waitForInternetEgress(configuration: configuration, port: port)
        egressMetadata = TunnelEgressMetadata(
            pingMs: egressProbe.durationMs,
            target: egressProbe.target,
            countryCode: egressProbe.countryCode,
            countryName: egressProbe.countryName
        )

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

        if configuration.usesCustomCarrier {
            NSLog("[PHANTOM] EXT: Config - stack=directsock carrier='\(configuration.carrierProtocolLabel)' uri='\(configuration.executableCarrierURI)' localPort='\(configuration.carrierProxyPort)'")
            SharedTunnelLogStore.append("[EXT] Config loaded: \(configuration.carrierProtocolLabel)=\(configuration.endpointHost):\(configuration.endpointPort) port=\(configuration.carrierProxyPort)")
        } else if configuration.usesPsiphonChain {
            let upstreamLabel = configuration.normalizedUpstreamProxy.isEmpty ? "(dynamic)" : redactedProxy(configuration.normalizedUpstreamProxy)
            NSLog("[PHANTOM] EXT: Config - stack=psiphon_chain carrier='\(configuration.endpointHost):\(configuration.endpointPort)' packetURL='\(configuration.normalizedServerURL)' upstream='\(upstreamLabel)'")
            SharedTunnelLogStore.append("[EXT] Config loaded: Psiphon Chain carrier=\(configuration.endpointHost):\(configuration.endpointPort) packet=\(configuration.normalizedServerURL) upstream=\(upstreamLabel)")
        } else if configuration.usesPacketChain {
            NSLog("[PHANTOM] EXT: Config - stack=packet_chain carrier='\(configuration.endpointHost):\(configuration.endpointPort)' packetURL='\(configuration.normalizedServerURL)'")
            SharedTunnelLogStore.append("[EXT] Config loaded: Packet Chain carrier=\(configuration.endpointHost):\(configuration.endpointPort) packet=\(configuration.normalizedServerURL)")
        } else {
            let upstreamLabel = configuration.normalizedUpstreamProxy.isEmpty ? "(none)" : redactedProxy(configuration.normalizedUpstreamProxy)
            NSLog("[PHANTOM] EXT: Config - URL='\(configuration.normalizedServerURL)' secretLen=\(configuration.normalizedSecret.count) port='\(configuration.listenPort)' transport=\(configuration.transportMode.title) upstream='\(upstreamLabel)'")
            SharedTunnelLogStore.append("[EXT] Config loaded: URL='\(configuration.normalizedServerURL)' transport=\(configuration.transportMode.title) port=\(configuration.listenPort) upstream=\(upstreamLabel)")
        }

        if let validationError = configuration.validationError {
            NSLog("[PHANTOM] EXT: ❌ Invalid advanced tunnel configuration")
            SharedTunnelLogStore.append("[EXT] ❌ \(validationError)")
            throw PacketTunnelError.invalidConfiguration(validationError)
        }

        return configuration
    }

    private func startRuntime(for configuration: TunnelConfiguration) async throws -> Int32 {
        guard configuration.usesPsiphonChain else {
            return TunnelRuntimeBridge.startRustClient(with: configuration)
        }

        let carrierPort = TunnelRuntimeBridge.startLayeredCarrier(with: configuration)
        guard carrierPort > 0 else {
            SharedTunnelLogStore.append(
                "[EXT] ❌ DirectSock carrier failed before Psiphon chain (code \(carrierPort))"
            )
            return carrierPort
        }

        let psiphonUpstream = "http://127.0.0.1:\(carrierPort)"
        SharedTunnelLogStore.append(
            "[EXT] DirectSock carrier is ready on 127.0.0.1:\(carrierPort); starting Psiphon through it"
        )

        let psiphon = try await PacketPsiphonCore.shared.startLocalProxy(
            upstreamProxyURL: psiphonUpstream,
            requestedHTTPPort: PacketDefaultProfiles.psiphonLocalHTTPPort,
            requestedSocksPort: PacketDefaultProfiles.psiphonLocalSocksPort
        )

        let packetUpstream = "http://127.0.0.1:\(psiphon.httpPort)"
        SharedTunnelLogStore.append(
            "[EXT] Psiphon HTTP proxy is ready on 127.0.0.1:\(psiphon.httpPort); starting Packet through Psiphon"
        )

        return TunnelRuntimeBridge.startRustClient(
            with: configuration,
            upstreamProxyOverride: packetUpstream
        )
    }

    private func requestedListenPort(from configuration: TunnelConfiguration) throws -> UInt16 {
        if configuration.usesCustomCarrier {
            guard let port = configuration.carrierProxyPortValue else {
                SharedTunnelLogStore.append("[EXT] ❌ Invalid DirectSock local port '\(configuration.carrierProxyPort)'")
                throw PacketTunnelError.invalidConfiguration("DirectSock local port must be 1024-65535.")
            }

            return port
        }

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

        // The Rust core exposes a local proxy. Do not advertise a full-device
        // default route until packetFlow is bridged.
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
        proxySettings.proxyAutoConfigurationJavaScript = makeProxyAutoConfigurationScript(
            for: configuration,
            port: port
        )
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
                SharedTunnelLogStore.append("[EXT] Local proxy listener is ready (attempt \(attempts))")
                return
            }
            if attempts % 5 == 0 {
                SharedTunnelLogStore.append("[EXT] Still waiting for local proxy... (\(attempts) attempts)")
            }
            try await Task.sleep(nanoseconds: 200_000_000)
        }

        SharedTunnelLogStore.append("[EXT] ❌ Timed out waiting for local proxy on port \(port) after 10s")
        throw PacketTunnelError.proxyNotReady
    }

    private func waitForInternetEgress(
        configuration: TunnelConfiguration,
        port: UInt16
    ) async throws -> InternetEgressProbeResult {
        let startedAt = Date()
        let timeoutAt = startedAt.addingTimeInterval(300)
        var attempts = 0
        let label = configuration.usesCustomCarrier
            ? "DirectSock"
            : configuration.usesPsiphonChain ? "Packet Chain" : "tunnel"
        var lastProbe = InternetEgressProbeResult(
            succeeded: false,
            target: "none",
            detail: "Internet probe did not run.",
            durationMs: 0,
            countryCode: nil,
            countryName: nil
        )

        SharedTunnelLogStore.append(
            "[EXT] Waiting up to 300s for real internet egress through \(label) 127.0.0.1:\(port)"
        )

        while Date() < timeoutAt {
            try Task.checkCancellation()
            attempts += 1

            lastProbe = await probeInternetEgress(configuration: configuration, port: port)
            if lastProbe.succeeded {
                let countrySuffix = lastProbe.countryName.map { " country=\($0)" } ?? ""
                SharedTunnelLogStore.append(
                    "[EXT] Internet probe passed via \(lastProbe.target) in \(lastProbe.durationMs)ms\(countrySuffix)"
                )
                return lastProbe
            }

            let elapsed = Int(Date().timeIntervalSince(startedAt))
            let remaining = max(0, Int(timeoutAt.timeIntervalSinceNow))
            if attempts == 1 || attempts % 3 == 0 {
                SharedTunnelLogStore.append(
                    "[EXT] \(label) internet probe still waiting after \(elapsed)s: \(lastProbe.detail)"
                )
            }
            SharedTunnelLogStore.append("[EXT] Internet probe retry window remaining: \(remaining)s")
            try await Task.sleep(nanoseconds: 2_000_000_000)
        }

        throw PacketTunnelError.egressNotReady(
            "No HTTP response through \(label) within 300s. Last error: \(lastProbe.detail)"
        )
    }

    private func probeInternetEgress(
        configuration: TunnelConfiguration,
        port: UInt16
    ) async -> InternetEgressProbeResult {
        var failures: [String] = []
        var firstSuccessfulProbe: InternetEgressProbeResult?

        for target in egressProbeTargets {
            let startedAt = Date()
            do {
                let preview: String
                if configuration.usesCustomCarrier {
                    preview = try requestHTTPThroughCarrierProxy(port: port, target: target)
                } else {
                    preview = try requestHTTPThroughSocksProxy(port: port, target: target)
                }
                let country = parseProbeCountry(host: target.host, preview: preview)
                let result = InternetEgressProbeResult(
                    succeeded: true,
                    target: "\(target.host):80",
                    detail: preview,
                    durationMs: Int(Date().timeIntervalSince(startedAt) * 1000),
                    countryCode: country.code,
                    countryName: country.name
                )
                if result.countryCode != nil || result.countryName != nil {
                    return result
                }
                if firstSuccessfulProbe == nil {
                    firstSuccessfulProbe = result
                }
            } catch {
                failures.append("\(target.host):80 -> \(error.localizedDescription)")
            }
        }

        if let firstSuccessfulProbe {
            return firstSuccessfulProbe
        }

        return InternetEgressProbeResult(
            succeeded: false,
            target: egressProbeTargets.map { "\($0.host):80" }.joined(separator: ", "),
            detail: failures.joined(separator: " | "),
            durationMs: 0,
            countryCode: nil,
            countryName: nil
        )
    }

    private func requestHTTPThroughCarrierProxy(
        port: UInt16,
        target: EgressProbeTarget
    ) throws -> String {
        let socketDescriptor = socket(AF_INET, SOCK_STREAM, 0)
        guard socketDescriptor >= 0 else {
            throw carrierProbeError("socket failed: \(errnoDescription())")
        }
        defer { close(socketDescriptor) }

        setSocketTimeouts(socketDescriptor, seconds: 10)

        var address = sockaddr_in()
        address.sin_len = UInt8(MemoryLayout<sockaddr_in>.stride)
        address.sin_family = sa_family_t(AF_INET)
        address.sin_port = port.bigEndian

        let conversionResult = "127.0.0.1".withCString { pointer in
            inet_pton(AF_INET, pointer, &address.sin_addr)
        }
        guard conversionResult == 1 else {
            throw carrierProbeError("failed to encode loopback address")
        }

        let connectResult = withUnsafePointer(to: &address) { pointer in
            pointer.withMemoryRebound(to: sockaddr.self, capacity: 1) { sockaddrPointer in
                connect(
                    socketDescriptor,
                    sockaddrPointer,
                    socklen_t(MemoryLayout<sockaddr_in>.stride)
                )
            }
        }
        guard connectResult == 0 else {
            throw carrierProbeError("connect to 127.0.0.1:\(port) failed: \(errnoDescription())")
        }

        let request = "GET http://\(target.host)\(target.path) HTTP/1.1\r\n"
            + "Host: \(target.host)\r\n"
            + "User-Agent: PacketCarrierProbe/1.0\r\n"
            + "Connection: close\r\n"
            + "\r\n"
        try sendAll(request, socketDescriptor: socketDescriptor)

        return try readHTTPResponsePreview(socketDescriptor: socketDescriptor)
    }

    private func requestHTTPThroughSocksProxy(
        port: UInt16,
        target: EgressProbeTarget
    ) throws -> String {
        let socketDescriptor = socket(AF_INET, SOCK_STREAM, 0)
        guard socketDescriptor >= 0 else {
            throw carrierProbeError("socket failed: \(errnoDescription())")
        }
        defer { close(socketDescriptor) }

        setSocketTimeouts(socketDescriptor, seconds: 10)

        var address = sockaddr_in()
        address.sin_len = UInt8(MemoryLayout<sockaddr_in>.stride)
        address.sin_family = sa_family_t(AF_INET)
        address.sin_port = port.bigEndian

        let conversionResult = "127.0.0.1".withCString { pointer in
            inet_pton(AF_INET, pointer, &address.sin_addr)
        }
        guard conversionResult == 1 else {
            throw carrierProbeError("failed to encode loopback address")
        }

        let connectResult = withUnsafePointer(to: &address) { pointer in
            pointer.withMemoryRebound(to: sockaddr.self, capacity: 1) { sockaddrPointer in
                connect(
                    socketDescriptor,
                    sockaddrPointer,
                    socklen_t(MemoryLayout<sockaddr_in>.stride)
                )
            }
        }
        guard connectResult == 0 else {
            throw carrierProbeError("connect to 127.0.0.1:\(port) failed: \(errnoDescription())")
        }

        try sendAll([0x05, 0x01, 0x00], socketDescriptor: socketDescriptor)
        let authReply = try readExact(byteCount: 2, socketDescriptor: socketDescriptor)
        guard authReply == [0x05, 0x00] else {
            throw carrierProbeError("SOCKS5 auth failed")
        }

        let hostBytes = Array(target.host.utf8)
        guard hostBytes.count <= 255 else {
            throw carrierProbeError("SOCKS5 target host is too long")
        }

        var connectRequest: [UInt8] = [0x05, 0x01, 0x00, 0x03, UInt8(hostBytes.count)]
        connectRequest.append(contentsOf: hostBytes)
        connectRequest.append(UInt8((target.port >> 8) & 0xff))
        connectRequest.append(UInt8(target.port & 0xff))
        try sendAll(connectRequest, socketDescriptor: socketDescriptor)

        let connectReply = try readExact(byteCount: 4, socketDescriptor: socketDescriptor)
        guard connectReply[0] == 0x05 else {
            throw carrierProbeError("invalid SOCKS5 reply")
        }
        guard connectReply[1] == 0x00 else {
            throw carrierProbeError("SOCKS5 CONNECT failed with code \(connectReply[1])")
        }

        try consumeSocksBindAddress(type: connectReply[3], socketDescriptor: socketDescriptor)

        let request = "GET \(target.path) HTTP/1.1\r\n"
            + "Host: \(target.host)\r\n"
            + "User-Agent: PacketEgressProbe/1.0\r\n"
            + "Connection: close\r\n"
            + "\r\n"
        try sendAll(request, socketDescriptor: socketDescriptor)

        return try readHTTPResponsePreview(socketDescriptor: socketDescriptor)
    }

    private func readHTTPResponsePreview(socketDescriptor: Int32) throws -> String {
        var buffer = [UInt8](repeating: 0, count: 1024)
        let bufferCapacity = buffer.count
        let bytesRead = buffer.withUnsafeMutableBytes { rawBuffer in
            recv(socketDescriptor, rawBuffer.baseAddress, bufferCapacity, 0)
        }
        guard bytesRead > 0 else {
            throw carrierProbeError("proxy returned no HTTP bytes: \(errnoDescription())")
        }

        let preview = String(decoding: buffer.prefix(bytesRead), as: UTF8.self)
            .replacingOccurrences(of: "\n", with: " ")
            .replacingOccurrences(of: "\r", with: " ")
        guard preview.hasPrefix("HTTP/") else {
            throw carrierProbeError("non-HTTP response: \(String(preview.prefix(80)))")
        }

        return String(preview.prefix(512))
    }

    private func sendAll(_ value: String, socketDescriptor: Int32) throws {
        try sendAll(Array(value.utf8), socketDescriptor: socketDescriptor)
    }

    private func sendAll(_ bytes: [UInt8], socketDescriptor: Int32) throws {
        var sent = 0

        while sent < bytes.count {
            let result = bytes.withUnsafeBytes { rawBuffer in
                guard let baseAddress = rawBuffer.baseAddress else { return -1 }
                return send(
                    socketDescriptor,
                    baseAddress.advanced(by: sent),
                    bytes.count - sent,
                    0
                )
            }

            guard result > 0 else {
                throw carrierProbeError("send failed: \(errnoDescription())")
            }

            sent += result
        }
    }

    private func readExact(byteCount: Int, socketDescriptor: Int32) throws -> [UInt8] {
        var buffer = [UInt8](repeating: 0, count: byteCount)
        var received = 0

        while received < byteCount {
            let result = buffer.withUnsafeMutableBytes { rawBuffer in
                guard let baseAddress = rawBuffer.baseAddress else { return -1 }
                return recv(
                    socketDescriptor,
                    baseAddress.advanced(by: received),
                    byteCount - received,
                    0
                )
            }

            guard result > 0 else {
                throw carrierProbeError("read failed: \(errnoDescription())")
            }

            received += result
        }

        return buffer
    }

    private func consumeSocksBindAddress(type addressType: UInt8, socketDescriptor: Int32) throws {
        switch addressType {
        case 0x01:
            _ = try readExact(byteCount: 6, socketDescriptor: socketDescriptor)
        case 0x03:
            let length = try readExact(byteCount: 1, socketDescriptor: socketDescriptor)[0]
            _ = try readExact(byteCount: Int(length) + 2, socketDescriptor: socketDescriptor)
        case 0x04:
            _ = try readExact(byteCount: 18, socketDescriptor: socketDescriptor)
        default:
            throw carrierProbeError("unsupported SOCKS5 address type \(addressType)")
        }
    }

    private func parseTraceCountryCode(from preview: String) -> String? {
        guard let range = preview.range(of: "loc=") else {
            return nil
        }

        let suffix = preview[range.upperBound...]
        let code = suffix.prefix { $0.isLetter }
        guard code.count == 2 else {
            return nil
        }

        return code.uppercased()
    }

    private func parseProbeCountry(host: String, preview: String) -> ProbeCountry {
        if let countryCode = parseTraceCountryCode(from: preview) {
            return ProbeCountry(
                code: countryCode,
                name: countryName(for: countryCode)
            )
        }

        guard host.caseInsensitiveCompare("ip-api.com") == .orderedSame else {
            return ProbeCountry(code: nil, name: nil)
        }

        let tokens = preview.split { $0.isWhitespace }.map(String.init)
        guard let codeIndex = tokens.firstIndex(where: { token in
            token.count == 2
                && token != "OK"
                && token.allSatisfy(\.isLetter)
                && token.uppercased() == token
        }) else {
            return ProbeCountry(code: nil, name: nil)
        }

        let code = tokens[codeIndex].uppercased()
        let name = codeIndex > 0 && tokens[codeIndex - 1].count > 2
            ? tokens[codeIndex - 1]
            : countryName(for: code)
        return ProbeCountry(code: code, name: name)
    }

    private func countryName(for code: String) -> String? {
        Locale.current.localizedString(forRegionCode: code.uppercased())
    }

    private func setSocketTimeouts(_ socketDescriptor: Int32, seconds: Int) {
        var timeout = timeval(tv_sec: seconds, tv_usec: 0)
        withUnsafePointer(to: &timeout) { pointer in
            pointer.withMemoryRebound(to: UInt8.self, capacity: MemoryLayout<timeval>.size) { timeoutPointer in
                _ = setsockopt(
                    socketDescriptor,
                    SOL_SOCKET,
                    SO_RCVTIMEO,
                    timeoutPointer,
                    socklen_t(MemoryLayout<timeval>.size)
                )
                _ = setsockopt(
                    socketDescriptor,
                    SOL_SOCKET,
                    SO_SNDTIMEO,
                    timeoutPointer,
                    socklen_t(MemoryLayout<timeval>.size)
                )
            }
        }
    }

    private func carrierProbeError(_ message: String) -> NSError {
        NSError(
            domain: "PacketCarrierProbe",
            code: Int(errno),
            userInfo: [NSLocalizedDescriptionKey: message]
        )
    }

    private func errnoDescription() -> String {
        if errno == 0 {
            return "no errno"
        }

        return String(cString: strerror(errno))
    }

    private func redactedProxy(_ value: String) -> String {
        guard var components = URLComponents(string: value) else {
            return "(invalid)"
        }
        if components.user != nil {
            components.user = "user"
        }
        if components.password != nil {
            components.password = "redacted"
        }
        return components.string ?? "(invalid)"
    }

    private func makeProxyAutoConfigurationScript(
        for configuration: TunnelConfiguration,
        port: UInt16
    ) -> String {
        let proxyDirective = configuration.usesCustomCarrier
            ? "PROXY 127.0.0.1:\(port)"
            : "SOCKS5 127.0.0.1:\(port); SOCKS 127.0.0.1:\(port)"

        return """
        function FindProxyForURL(url, host) {
            return "\(proxyDirective); DIRECT";
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

    private struct EgressProbeTarget {
        let host: String
        let path: String
        let port: UInt16 = 80
    }

    private struct InternetEgressProbeResult {
        let succeeded: Bool
        let target: String
        let detail: String
        let durationMs: Int
        let countryCode: String?
        let countryName: String?
    }

    private struct ProbeCountry {
        let code: String?
        let name: String?
    }

    private struct TunnelEgressMetadata {
        var pingMs: Int?
        var target: String?
        var countryCode: String?
        var countryName: String?

        static let empty = TunnelEgressMetadata()
    }
}

private enum PacketTunnelError: LocalizedError {
    case missingConfiguration
    case invalidConfiguration(String)
    case rustStartFailed(Int32)
    case proxyNotReady
    case egressNotReady(String)

    var errorDescription: String? {
        switch self {
        case .missingConfiguration:
            return "Missing NetworkExtension provider configuration."
        case let .invalidConfiguration(message):
            return message
        case let .rustStartFailed(code):
            return "Rust tunnel core failed to start with code \(code)."
        case .proxyNotReady:
            return "The local proxy listener did not become ready in time."
        case let .egressNotReady(message):
            return message
        }
    }
}
