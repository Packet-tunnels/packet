import Foundation

enum TunnelConstants {
    static let localizedDescription = "Packet"
    static let providerBundleIdentifier = "com.resolo.packet.PacketTunnel"
    static let legacyProviderBundleIdentifiers = ["com.resolo.phantom.PacketTunnel", "com.resolo.phantom.PhantomTunnel.PacketTunnel"]
    static let legacyLocalizedDescriptions = ["Phantom Tunnel", "PhantomTunnel"]
    static let appGroupIdentifier = "group.com.resolo.packet.tunnel"
    static let sharedLogsKey = "shared.logs"
    static let maxLogEntries = 200
}

enum SharedTunnelPreferenceKeys {
    static let vpnDisclosureAcknowledged = "vpnDisclosureAcknowledged"
}

enum TunnelProviderKeys {
    static let serverURL = "serverURL"
    static let secret = "secret"
    static let listenPort = "listenPort"
    static let cdnEdge = "cdnEdge"
    static let hostOverride = "hostOverride"
    static let transportMode = "transportMode"
}

enum TunnelTransportMode: Int32, CaseIterable, Identifiable {
    case auto = 0
    case webSocket = 1
    case http = 2

    var id: Int32 { rawValue }

    var title: String {
        switch self {
        case .auto:
            return "Auto"
        case .webSocket:
            return "WebSocket"
        case .http:
            return "HTTP"
        }
    }
}

struct TunnelConfiguration: Equatable {
    var serverURL = ""
    var secret = ""
    var listenPort = ""
    var cdnEdge = ""
    var hostOverride = ""
    var transportMode: TunnelTransportMode = .auto

    init() {}

    init(providerConfiguration: [String: Any]) {
        serverURL = providerConfiguration[TunnelProviderKeys.serverURL] as? String ?? serverURL
        secret = providerConfiguration[TunnelProviderKeys.secret] as? String ?? secret
        cdnEdge = providerConfiguration[TunnelProviderKeys.cdnEdge] as? String ?? cdnEdge
        hostOverride = providerConfiguration[TunnelProviderKeys.hostOverride] as? String ?? hostOverride

        if let port = providerConfiguration[TunnelProviderKeys.listenPort] as? NSNumber,
            port.intValue > 0
        {
            listenPort = port.stringValue
        } else if let port = providerConfiguration[TunnelProviderKeys.listenPort] as? String {
            listenPort = port
        }

        if let rawValue = providerConfiguration[TunnelProviderKeys.transportMode] as? NSNumber {
            transportMode = TunnelTransportMode(rawValue: rawValue.int32Value) ?? .auto
        } else if let rawValue = providerConfiguration[TunnelProviderKeys.transportMode] as? Int32 {
            transportMode = TunnelTransportMode(rawValue: rawValue) ?? .auto
        }
    }

    var normalizedServerURL: String {
        Self.trimmed(serverURL)
    }

    var normalizedSecret: String {
        Self.trimmed(secret)
    }

    var normalizedCDNEdge: String {
        Self.trimmed(cdnEdge)
    }

    var normalizedHostOverride: String {
        Self.trimmed(hostOverride)
    }

    var cdnEdgeValidationError: String? {
        let edge = normalizedCDNEdge
        if edge.isEmpty {
            return nil
        }

        if edge.allSatisfy(\.isNumber) {
            return "CDN edge must be a host or IP, optionally with :port. If you only need a custom origin port, add it to Server URL instead."
        }

        if edge.hasPrefix(":") || edge.hasSuffix(":") {
            return "CDN edge must look like 185.143.234.235:80 or edge.example.ir."
        }

        let parts = edge.split(separator: ":", omittingEmptySubsequences: false)
        if parts.count == 2 {
            let port = String(parts[1])
            guard let portValue = Int(port), (1...65535).contains(portValue) else {
                return "CDN edge port must be between 1 and 65535."
            }
        }

        return nil
    }

    var usesCDN: Bool {
        !normalizedCDNEdge.isEmpty || !normalizedHostOverride.isEmpty
    }

    var listenPortValue: UInt16? {
        let trimmed = Self.trimmed(listenPort)
        if trimmed.isEmpty || trimmed.lowercased() == "auto" {
            return nil
        }

        return UInt16(trimmed)
    }

    var remoteAddress: String {
        if let host = URL(string: normalizedServerURL)?.host, !host.isEmpty {
            return host
        }

        return normalizedServerURL
            .replacingOccurrences(of: "http://", with: "")
            .replacingOccurrences(of: "https://", with: "")
            .split(separator: "/")
            .first
            .map(String.init) ?? "127.0.0.1"
    }

    var ingressLabel: String {
        if !normalizedCDNEdge.isEmpty || !normalizedHostOverride.isEmpty {
            return "CDN relay"
        }

        return "Standard endpoint"
    }

    var endpointHost: String {
        if cdnEdgeValidationError != nil {
            return remoteAddress
        }

        let edgeHost = normalizedCDNEdge
            .split(separator: ":")
            .first
            .map(String.init)?
            .trimmingCharacters(in: .whitespacesAndNewlines) ?? ""

        return edgeHost.isEmpty ? remoteAddress : edgeHost
    }

    var endpointPort: Int {
        if cdnEdgeValidationError == nil {
            let edgeParts = normalizedCDNEdge.split(separator: ":")
            if edgeParts.count == 2, let port = Int(edgeParts[1]), (1...65535).contains(port) {
                return port
            }
        }

        if let port = URL(string: normalizedServerURL)?.port, port > 0 {
            return port
        }

        return normalizedServerURL.lowercased().hasPrefix("https://") ? 443 : 80
    }

    var proxyExceptionHosts: [String] {
        var hosts = ["127.0.0.1", "localhost", remoteAddress]

        if !normalizedCDNEdge.isEmpty {
            let edgeHost = normalizedCDNEdge
                .split(separator: ":")
                .first
                .map(String.init)

            if let edgeHost, !edgeHost.isEmpty {
                hosts.append(edgeHost)
            }
        }

        return Array(Set(hosts.filter { !$0.isEmpty }))
    }

    var providerConfiguration: [String: Any] {
        var configuration: [String: Any] = [
            TunnelProviderKeys.serverURL: normalizedServerURL,
            TunnelProviderKeys.secret: normalizedSecret,
            TunnelProviderKeys.cdnEdge: normalizedCDNEdge,
            TunnelProviderKeys.hostOverride: normalizedHostOverride,
            TunnelProviderKeys.transportMode: Int(transportMode.rawValue)
        ]

        if let listenPortValue {
            configuration[TunnelProviderKeys.listenPort] = Int(listenPortValue)
        }

        return configuration
    }

    private static func trimmed(_ value: String) -> String {
        value.trimmingCharacters(in: .whitespacesAndNewlines)
    }
}

struct TunnelRuntimeSnapshot: Decodable, Equatable {
    var state = "idle"
    var transport = "Auto"
    var serverHost = ""
    var cdnEdge: String?
    var listenPort: UInt16?
    var bytesUp: UInt64 = 0
    var bytesDown: UInt64 = 0
    var activeStreams: UInt32 = 0
    var totalStreams: UInt64 = 0
    var connectedSince: UInt64?
    var lastPingMs: UInt32?
    var lastError: String?
    var tunnelActive = false

    static let empty = TunnelRuntimeSnapshot()

    var endpointHost: String {
        if let cdnEdge, !cdnEdge.isEmpty {
            return cdnEdge
        }

        return serverHost
    }
}

enum SharedTunnelPreferenceStore {
    private static var defaults: UserDefaults {
        UserDefaults(suiteName: TunnelConstants.appGroupIdentifier) ?? .standard
    }

    static var vpnDisclosureAcknowledged: Bool {
        defaults.bool(forKey: SharedTunnelPreferenceKeys.vpnDisclosureAcknowledged)
    }

    static func setVPNDisclosureAcknowledged(_ acknowledged: Bool) {
        defaults.set(acknowledged, forKey: SharedTunnelPreferenceKeys.vpnDisclosureAcknowledged)
    }
}

enum SharedTunnelLogStore {
    private static let queue = DispatchQueue(label: "com.resolo.packet.shared-log-store")

    private static var defaults: UserDefaults {
        UserDefaults(suiteName: TunnelConstants.appGroupIdentifier) ?? .standard
    }

    static func load() -> [String] {
        queue.sync {
            defaults.stringArray(forKey: TunnelConstants.sharedLogsKey) ?? []
        }
    }

    static func append(_ message: String) {
        let trimmed = message.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }

        queue.sync {
            var logs = defaults.stringArray(forKey: TunnelConstants.sharedLogsKey) ?? []
            logs.append(trimmed)

            if logs.count > TunnelConstants.maxLogEntries {
                logs.removeFirst(logs.count - TunnelConstants.maxLogEntries)
            }

            defaults.set(logs, forKey: TunnelConstants.sharedLogsKey)
        }
    }

    static func clear() {
        queue.sync {
            defaults.removeObject(forKey: TunnelConstants.sharedLogsKey)
        }
    }
}

private func rustLogCallback(_ cString: UnsafePointer<CChar>?) {
    guard let cString else { return }
    SharedTunnelLogStore.append(String(cString: cString))
}

enum TunnelRuntimeBridge {
    private static var installedLogCallback = false

    static func installLogCallback() {
        guard !installedLogCallback else { return }
        installedLogCallback = true
        phantom_set_log_callback(rustLogCallback)
    }

    static func startRustClient(with configuration: TunnelConfiguration) -> Int32 {
        installLogCallback()
        let listenPort = configuration.listenPortValue ?? 0

        return configuration.normalizedServerURL.withCString { serverURLPointer in
            configuration.normalizedSecret.withCString { secretPointer in
                if configuration.usesCDN || configuration.transportMode != .auto {
                    return withOptionalCString(configuration.normalizedCDNEdge) { cdnEdgePointer in
                        withOptionalCString(configuration.normalizedHostOverride) { hostOverridePointer in
                            phantom_start_cdn(
                                serverURLPointer,
                                secretPointer,
                                listenPort,
                                cdnEdgePointer,
                                hostOverridePointer,
                                configuration.transportMode.rawValue
                            )
                        }
                    }
                }

                return phantom_start(serverURLPointer, secretPointer, listenPort)
            }
        }
    }

    static func runtimeSnapshot() -> TunnelRuntimeSnapshot? {
        guard let jsonData = runtimeStatsData() else {
            return nil
        }

        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        return try? decoder.decode(TunnelRuntimeSnapshot.self, from: jsonData)
    }

    static func runtimeStatsData() -> Data? {
        guard let rawPointer = phantom_copy_stats_json() else {
            return nil
        }
        defer {
            phantom_free_string(rawPointer)
        }

        return String(cString: rawPointer).data(using: .utf8)
    }

    private static func withOptionalCString<T>(
        _ value: String,
        body: (UnsafePointer<CChar>?) -> T
    ) -> T {
        if value.isEmpty {
            return body(nil)
        }

        return value.withCString(body)
    }
}
