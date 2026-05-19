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
    static let savedConfigurations = "savedConfigurations"
    static let selectedConfigurationID = "selectedConfigurationID"
    static let activeConfigurationID = "activeConfigurationID"
}

enum TunnelProviderKeys {
    static let stackMode = "stackMode"
    static let serverURL = "serverURL"
    static let secret = "secret"
    static let listenPort = "listenPort"
    static let cdnEdge = "cdnEdge"
    static let hostOverride = "hostOverride"
    static let sniOverride = "sniOverride"
    static let transportMode = "transportMode"
    static let obfsKey = "obfsKey"
    static let fragmentEnabled = "fragmentEnabled"
    static let fragmentSize = "fragmentSize"
    static let trojanCarrierURI = "trojanCarrierURI"
    static let carrierProxyPort = "carrierProxyPort"
}

enum TunnelStackMode: Int32, CaseIterable, Identifiable, Codable {
    case packetNative = 0
    case customTrojanCarrier = 1

    var id: Int32 { rawValue }

    var title: String {
        switch self {
        case .packetNative:
            return "Packet Native"
        case .customTrojanCarrier:
            return "DirectSock"
        }
    }
}

enum TunnelTransportMode: Int32, CaseIterable, Identifiable, Codable {
    case auto = 0
    case webSocket = 1
    case http = 2
    case stealth = 3
    case obfs = 4

    var id: Int32 { rawValue }

    var title: String {
        switch self {
        case .auto:
            return "Auto"
        case .webSocket:
            return "WebSocket"
        case .http:
            return "HTTP"
        case .stealth:
            return "Stealth"
        case .obfs:
            return "Obfs"
        }
    }
}

struct TunnelConfiguration: Codable, Equatable {
    var stackMode: TunnelStackMode = .packetNative
    var serverURL = ""
    var secret = ""
    var listenPort = ""
    var cdnEdge = ""
    var hostOverride = ""
    var sniOverride = ""
    var transportMode: TunnelTransportMode = .auto
    var obfsKey = ""
    var fragmentEnabled = true
    var fragmentSize = "40"
    var trojanCarrierURI = ""
    var carrierProxyPort = "10808"

    private enum CodingKeys: String, CodingKey {
        case stackMode
        case serverURL
        case secret
        case listenPort
        case cdnEdge
        case hostOverride
        case sniOverride
        case transportMode
        case obfsKey
        case fragmentEnabled
        case fragmentSize
        case trojanCarrierURI
        case carrierProxyPort
    }

    init() {}

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        stackMode = try container.decodeIfPresent(TunnelStackMode.self, forKey: .stackMode) ?? .packetNative
        serverURL = try container.decodeIfPresent(String.self, forKey: .serverURL) ?? ""
        secret = try container.decodeIfPresent(String.self, forKey: .secret) ?? ""
        listenPort = try container.decodeIfPresent(String.self, forKey: .listenPort) ?? ""
        cdnEdge = try container.decodeIfPresent(String.self, forKey: .cdnEdge) ?? ""
        hostOverride = try container.decodeIfPresent(String.self, forKey: .hostOverride) ?? ""
        sniOverride = try container.decodeIfPresent(String.self, forKey: .sniOverride) ?? ""
        transportMode = try container.decodeIfPresent(TunnelTransportMode.self, forKey: .transportMode) ?? .auto
        obfsKey = try container.decodeIfPresent(String.self, forKey: .obfsKey) ?? ""
        fragmentEnabled = try container.decodeIfPresent(Bool.self, forKey: .fragmentEnabled) ?? true
        fragmentSize = try container.decodeIfPresent(String.self, forKey: .fragmentSize) ?? "40"
        trojanCarrierURI = try container.decodeIfPresent(String.self, forKey: .trojanCarrierURI) ?? ""
        carrierProxyPort = try container.decodeIfPresent(String.self, forKey: .carrierProxyPort) ?? "10808"
    }

    init(providerConfiguration: [String: Any]) {
        if let rawValue = providerConfiguration[TunnelProviderKeys.stackMode] as? NSNumber {
            stackMode = TunnelStackMode(rawValue: rawValue.int32Value) ?? .packetNative
        } else if let rawValue = providerConfiguration[TunnelProviderKeys.stackMode] as? Int32 {
            stackMode = TunnelStackMode(rawValue: rawValue) ?? .packetNative
        } else if let rawValue = providerConfiguration[TunnelProviderKeys.stackMode] as? Int {
            stackMode = TunnelStackMode(rawValue: Int32(rawValue)) ?? .packetNative
        }

        serverURL = providerConfiguration[TunnelProviderKeys.serverURL] as? String ?? serverURL
        secret = providerConfiguration[TunnelProviderKeys.secret] as? String ?? secret
        cdnEdge = providerConfiguration[TunnelProviderKeys.cdnEdge] as? String ?? cdnEdge
        hostOverride = providerConfiguration[TunnelProviderKeys.hostOverride] as? String ?? hostOverride
        sniOverride = providerConfiguration[TunnelProviderKeys.sniOverride] as? String ?? sniOverride
        obfsKey = providerConfiguration[TunnelProviderKeys.obfsKey] as? String ?? obfsKey
        trojanCarrierURI = providerConfiguration[TunnelProviderKeys.trojanCarrierURI] as? String ?? trojanCarrierURI
        carrierProxyPort = providerConfiguration[TunnelProviderKeys.carrierProxyPort] as? String ?? carrierProxyPort

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

        if let enabled = providerConfiguration[TunnelProviderKeys.fragmentEnabled] as? NSNumber {
            fragmentEnabled = enabled.boolValue
        } else if let enabled = providerConfiguration[TunnelProviderKeys.fragmentEnabled] as? Bool {
            fragmentEnabled = enabled
        }

        if let size = providerConfiguration[TunnelProviderKeys.fragmentSize] as? NSNumber,
            size.intValue > 0
        {
            fragmentSize = size.stringValue
        } else if let size = providerConfiguration[TunnelProviderKeys.fragmentSize] as? String {
            fragmentSize = size
        }

        if let port = providerConfiguration[TunnelProviderKeys.carrierProxyPort] as? NSNumber,
            port.intValue > 0
        {
            carrierProxyPort = port.stringValue
        }

    }

    var isEmpty: Bool {
        stackMode == .packetNative
            && normalizedServerURL.isEmpty
            && normalizedSecret.isEmpty
            && Self.trimmed(listenPort).isEmpty
            && normalizedCDNEdge.isEmpty
            && normalizedHostOverride.isEmpty
            && normalizedSNIOverride.isEmpty
            && normalizedObfsKey.isEmpty
            && transportMode == .auto
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

    var normalizedSNIOverride: String {
        Self.trimmed(sniOverride)
    }

    var normalizedObfsKey: String {
        Self.trimmed(obfsKey)
    }

    var normalizedTrojanCarrierURI: String {
        Self.trimmed(trojanCarrierURI)
    }

    var usesCustomCarrier: Bool {
        stackMode == .customTrojanCarrier
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

    var advancedValidationError: String? {
        if usesCustomCarrier {
            return layeredValidationError
        }

        if cdnEdgeValidationError != nil {
            return cdnEdgeValidationError
        }

        if !normalizedSNIOverride.isEmpty
            && !normalizedServerURL.lowercased().hasPrefix("https://")
        {
            return "SNI override requires an https:// server URL."
        }

        if transportMode == .stealth
            && !normalizedServerURL.lowercased().hasPrefix("https://")
        {
            return "Stealth transport requires an https:// server URL."
        }

        if transportMode == .obfs && normalizedCDNEdge.isEmpty {
            return "Obfs transport requires CDN Edge set to the direct server IP:port, for example 103.241.67.247:36571."
        }

        if fragmentEnabled {
            let rawSize = Self.trimmed(fragmentSize)
            guard let size = UInt32(rawSize), (1...1000).contains(size) else {
                return "Fragment size must be between 1 and 1000."
            }
        }

        return nil
    }

    var validationError: String? {
        if usesCustomCarrier {
            return layeredValidationError
        }

        if normalizedServerURL.isEmpty {
            return "Server URL is required."
        }

        if normalizedSecret.isEmpty {
            return "Shared secret is required."
        }

        return advancedValidationError
    }

    private var layeredValidationError: String? {
        if normalizedTrojanCarrierURI.isEmpty {
            return "Trojan URI is required for DirectSock mode."
        }

        if !normalizedTrojanCarrierURI.lowercased().hasPrefix("trojan://") {
            return "DirectSock URI must start with trojan://."
        }

        guard carrierProxyPortValue != nil else {
            return "DirectSock local port must be 1024-65535."
        }

        return nil
    }

    var usesCDN: Bool {
        !normalizedCDNEdge.isEmpty || !normalizedHostOverride.isEmpty
    }

    var usesAdvancedStart: Bool {
        usesCustomCarrier ||
            usesCDN ||
            !normalizedSNIOverride.isEmpty ||
            !normalizedObfsKey.isEmpty ||
            transportMode != .auto ||
            fragmentEnabled
    }

    var listenPortValue: UInt16? {
        let trimmed = Self.trimmed(listenPort)
        if trimmed.isEmpty || trimmed.lowercased() == "auto" {
            return nil
        }

        return UInt16(trimmed)
    }

    var carrierProxyPortValue: UInt16? {
        Self.validatedPort(carrierProxyPort, range: 1024...65535)
    }

    var effectiveCarrierProxyPort: UInt16 {
        carrierProxyPortValue ?? 10808
    }

    var fragmentSizeValue: UInt32 {
        guard fragmentEnabled else { return 40 }
        let value = UInt32(Self.trimmed(fragmentSize)) ?? 40
        return min(max(value, 1), 1000)
    }

    var remoteAddress: String {
        if usesCustomCarrier {
            return carrierEndpointHost
        }

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
        if usesCustomCarrier {
            return "DirectSock"
        }

        if transportMode == .stealth {
            return "Stealth TLS"
        }

        if transportMode == .obfs {
            return "Obfs raw TCP"
        }

        if !normalizedSNIOverride.isEmpty {
            return "SNI fronting"
        }

        if !normalizedCDNEdge.isEmpty || !normalizedHostOverride.isEmpty {
            return "CDN relay"
        }

        return "Standard endpoint"
    }

    var suggestedName: String {
        if usesCustomCarrier {
            return "DirectSock"
        }

        if !normalizedHostOverride.isEmpty {
            return normalizedHostOverride
        }

        if !normalizedCDNEdge.isEmpty {
            let edgeHost = normalizedCDNEdge
                .split(separator: ":")
                .first
                .map(String.init)?
                .trimmingCharacters(in: .whitespacesAndNewlines)

            if let edgeHost, !edgeHost.isEmpty {
                return edgeHost
            }
        }

        if !normalizedServerURL.isEmpty {
            return remoteAddress
        }

        return "New Server"
    }

    var endpointHost: String {
        if usesCustomCarrier {
            return carrierEndpointHost
        }

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
        if usesCustomCarrier {
            return carrierEndpointPort
        }

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

    private var carrierEndpointHost: String {
        URL(string: normalizedTrojanCarrierURI)?.host?.nilIfEmpty ?? "Unavailable"
    }

    private var carrierEndpointPort: Int {
        if let port = URL(string: normalizedTrojanCarrierURI)?.port, port > 0 {
            return port
        }

        return 443
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
            TunnelProviderKeys.stackMode: Int(stackMode.rawValue),
            TunnelProviderKeys.serverURL: normalizedServerURL,
            TunnelProviderKeys.secret: normalizedSecret,
            TunnelProviderKeys.cdnEdge: normalizedCDNEdge,
            TunnelProviderKeys.hostOverride: normalizedHostOverride,
            TunnelProviderKeys.sniOverride: normalizedSNIOverride,
            TunnelProviderKeys.transportMode: Int(transportMode.rawValue),
            TunnelProviderKeys.obfsKey: normalizedObfsKey,
            TunnelProviderKeys.fragmentEnabled: fragmentEnabled,
            TunnelProviderKeys.fragmentSize: Int(fragmentSizeValue),
            TunnelProviderKeys.trojanCarrierURI: normalizedTrojanCarrierURI,
            TunnelProviderKeys.carrierProxyPort: Int(effectiveCarrierProxyPort)
        ]

        if let listenPortValue {
            configuration[TunnelProviderKeys.listenPort] = Int(listenPortValue)
        }

        return configuration
    }

    private static func trimmed(_ value: String) -> String {
        value.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private static func validatedPort(_ value: String, range: ClosedRange<UInt16>) -> UInt16? {
        let trimmedValue = trimmed(value)
        guard let port = UInt16(trimmedValue), range.contains(port) else {
            return nil
        }

        return port
    }
}

struct SavedTunnelConfiguration: Codable, Equatable, Identifiable {
    var id: UUID
    var name: String
    var configuration: TunnelConfiguration

    init(
        id: UUID = UUID(),
        name: String = "",
        configuration: TunnelConfiguration = TunnelConfiguration()
    ) {
        self.id = id
        self.name = name
        self.configuration = configuration
    }

    var trimmedName: String {
        name.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    var displayName: String {
        trimmedName.nilIfEmpty ?? configuration.suggestedName
    }

    var subtitle: String {
        if configuration.usesCustomCarrier {
            return configuration.normalizedTrojanCarrierURI.nilIfEmpty ?? "DirectSock not configured"
        }

        return configuration.normalizedServerURL.nilIfEmpty ?? "Not configured"
    }
}

struct TunnelRuntimeSnapshot: Decodable, Equatable {
    var state = "idle"
    var transport = "Auto"
    var serverHost = ""
    var cdnEdge: String?
    var serverCountryCode: String?
    var serverCountryName: String?
    var egressPingMs: UInt32?
    var egressTarget: String?
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

struct PacketComplianceSummaryItem: Identifiable, Equatable {
    let id: String
    let systemImage: String
    let title: String
    let detail: String
}

enum PacketComplianceCopy {
    static let reminderText =
        "Review and accept the in-app VPN disclosure before your first connection."
    static let disclosureIntro =
        "Packet creates an iOS VPN configuration and uses a packet tunnel profile to send tunnel traffic through the server you configure while connected."
    static let disclosureOutro =
        "You can disconnect at any time from Packet or from the system VPN controls in iOS Settings."
    static let settingsFooterText =
        "Packet stores configuration details locally on-device. Legal documents open in the public Packet pages."

    static let summaryItems: [PacketComplianceSummaryItem] = [
        PacketComplianceSummaryItem(
            id: "local-storage",
            systemImage: "lock.shield",
            title: "Secure Local Storage",
            detail:
                "Your server URL and settings stay on this device. Saved profile secrets are kept in the iOS Keychain and passed to the tunnel extension only when connecting."
        ),
        PacketComplianceSummaryItem(
            id: "traffic-handling",
            systemImage: "network",
            title: "Traffic Through Your Server",
            detail:
                "When you connect, the server you configure receives the traffic and connection metadata required to establish and operate the tunnel."
        ),
        PacketComplianceSummaryItem(
            id: "tracking",
            systemImage: "eye.slash",
            title: "No Ads or Trackers",
            detail:
                "Packet does not bundle advertising, analytics, or tracking SDKs."
        ),
    ]
}

extension String {
    var nilIfEmpty: String? {
        let trimmedValue = trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmedValue.isEmpty ? nil : trimmedValue
    }
}

enum SharedTunnelPreferenceStore {
    private static var defaults: UserDefaults {
        UserDefaults(suiteName: TunnelConstants.appGroupIdentifier) ?? .standard
    }

    private static var encoder: JSONEncoder {
        JSONEncoder()
    }

    private static var decoder: JSONDecoder {
        JSONDecoder()
    }

    static var vpnDisclosureAcknowledged: Bool {
        defaults.bool(forKey: SharedTunnelPreferenceKeys.vpnDisclosureAcknowledged)
    }

    static func setVPNDisclosureAcknowledged(_ acknowledged: Bool) {
        defaults.set(acknowledged, forKey: SharedTunnelPreferenceKeys.vpnDisclosureAcknowledged)
    }

    static var savedConfigurations: [SavedTunnelConfiguration] {
        guard
            let data = defaults.data(forKey: SharedTunnelPreferenceKeys.savedConfigurations),
            let configurations = try? decoder.decode([SavedTunnelConfiguration].self, from: data)
        else {
            return []
        }

        return configurations
    }

    static func setSavedConfigurations(_ configurations: [SavedTunnelConfiguration]) {
        guard let data = try? encoder.encode(configurations) else { return }
        defaults.set(data, forKey: SharedTunnelPreferenceKeys.savedConfigurations)
    }

    static var selectedConfigurationID: UUID? {
        guard
            let rawValue = defaults.string(forKey: SharedTunnelPreferenceKeys.selectedConfigurationID)
        else {
            return nil
        }

        return UUID(uuidString: rawValue)
    }

    static func setSelectedConfigurationID(_ id: UUID?) {
        if let id {
            defaults.set(id.uuidString, forKey: SharedTunnelPreferenceKeys.selectedConfigurationID)
        } else {
            defaults.removeObject(forKey: SharedTunnelPreferenceKeys.selectedConfigurationID)
        }
    }

    static var activeConfigurationID: UUID? {
        guard
            let rawValue = defaults.string(forKey: SharedTunnelPreferenceKeys.activeConfigurationID)
        else {
            return nil
        }

        return UUID(uuidString: rawValue)
    }

    static func setActiveConfigurationID(_ id: UUID?) {
        if let id {
            defaults.set(id.uuidString, forKey: SharedTunnelPreferenceKeys.activeConfigurationID)
        } else {
            defaults.removeObject(forKey: SharedTunnelPreferenceKeys.activeConfigurationID)
        }
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

        if configuration.usesCustomCarrier {
            return configuration.normalizedTrojanCarrierURI.withCString { uriPointer in
                phantom_start_layered_carrier_full(
                    uriPointer,
                    configuration.effectiveCarrierProxyPort,
                    configuration.fragmentEnabled ? 1 : 0,
                    configuration.fragmentSizeValue
                )
            }
        }

        let listenPort = configuration.listenPortValue ?? 0

        return configuration.normalizedServerURL.withCString { serverURLPointer in
            configuration.normalizedSecret.withCString { secretPointer in
                if configuration.usesAdvancedStart {
                    return withOptionalCString(configuration.normalizedCDNEdge) { cdnEdgePointer in
                        withOptionalCString(configuration.normalizedHostOverride) { hostOverridePointer in
                            withOptionalCString(configuration.normalizedSNIOverride) { sniOverridePointer in
                                withOptionalCString(configuration.normalizedObfsKey) { obfsKeyPointer in
                                    phantom_start_full(
                                        serverURLPointer,
                                        secretPointer,
                                        listenPort,
                                        cdnEdgePointer,
                                        hostOverridePointer,
                                        sniOverridePointer,
                                        configuration.transportMode.rawValue,
                                        configuration.fragmentEnabled ? 1 : 0,
                                        configuration.fragmentSizeValue,
                                        configuration.transportMode == .stealth ? 1 : 0,
                                        obfsKeyPointer
                                    )
                                }
                            }
                        }
                    }
                }

                return phantom_start(serverURLPointer, secretPointer, listenPort)
            }
        }
    }

    static func stopRustClient() {
        installLogCallback()
        phantom_stop_client()
        phantom_stop_layered_carrier()
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
