import Combine
import Foundation
import NetworkExtension

struct TunnelTelemetry: Equatable {
    var snapshot: TunnelRuntimeSnapshot = .empty
    var uploadRateBps: Double = 0
    var downloadRateBps: Double = 0

    static let empty = TunnelTelemetry()

    var countryLabel: String {
        if let name = snapshot.serverCountryName?.nilIfEmpty {
            return name
        }

        if let code = snapshot.serverCountryCode?.nilIfEmpty {
            return Locale.current.localizedString(forRegionCode: code.uppercased()) ?? code.uppercased()
        }

        let hostParts = snapshot.serverHost.lowercased().split(separator: ".")
        guard let tld = hostParts.last, tld.count == 2 else {
            return "Unknown"
        }

        return Locale.current.localizedString(forRegionCode: tld.uppercased()) ?? tld.uppercased()
    }

    var endpointLabel: String {
        let endpoint = snapshot.endpointHost
        return endpoint.isEmpty ? "Unavailable" : endpoint
    }

    var serverLabel: String {
        snapshot.serverHost.isEmpty ? "Unavailable" : snapshot.serverHost
    }

    var transportLabel: String {
        snapshot.transport.isEmpty ? "Unknown" : snapshot.transport
    }

    var isWaitingForTraffic: Bool {
        snapshot.tunnelActive && snapshot.bytesUp == 0 && snapshot.bytesDown == 0
            && snapshot.activeStreams == 0
    }
}

private struct RuntimeRateAnchor {
    let date: Date
    let bytesUp: UInt64
    let bytesDown: UInt64
}

@MainActor
final class TunnelManager: ObservableObject {
    enum State: String {
        case idle = "Idle"
        case launching = "Launching"
        case running = "Running"
        case failed = "Failed"
    }

    @Published var configuration = TunnelConfiguration()
    @Published private(set) var savedConfigurations: [SavedTunnelConfiguration] = []
    @Published private(set) var selectedConfigurationID: UUID?
    @Published private(set) var activeConfigurationID: UUID?
    @Published private(set) var state: State = .idle
    @Published private(set) var lastResult = "Not started"
    @Published private(set) var telemetry = TunnelTelemetry.empty

    let logManager: LogManager

    private var providerManager: NETunnelProviderManager?
    private var statusObserver: NSObjectProtocol?
    private var statsRefreshTask: Task<Void, Never>?
    private var rateAnchor: RuntimeRateAnchor?
    private var activeConfigurationSnapshot: TunnelConfiguration?
    private var activeConfigurationName: String?

    init(logManager: LogManager = .shared) {
        self.logManager = logManager
        observeVPNStatus()
        startStatsRefreshLoop()

        if logManager.logs.isEmpty {
            logManager.appendLog("[APP] Packet UI is ready")
        }

        Task {
            await loadSavedConfiguration()
        }
    }

    deinit {
        if let statusObserver {
            NotificationCenter.default.removeObserver(statusObserver)
        }

        statsRefreshTask?.cancel()
    }

    var isBusy: Bool {
        state == .launching
    }

    var isRunning: Bool {
        state == .running || state == .launching
    }

    var primaryActionTitle: String {
        isRunning ? "Disconnect Tunnel" : "Connect Tunnel"
    }

    var selectedConfiguration: SavedTunnelConfiguration? {
        guard let selectedConfigurationID else { return nil }
        return savedConfigurations.first { $0.id == selectedConfigurationID }
    }

    var activeConfiguration: SavedTunnelConfiguration? {
        guard let activeConfigurationID else { return nil }
        return savedConfigurations.first { $0.id == activeConfigurationID }
    }

    var displayConfiguration: TunnelConfiguration {
        if (isRunning || telemetry.snapshot.tunnelActive), let activeConfigurationSnapshot {
            return activeConfigurationSnapshot
        }

        return configuration
    }

    var selectedConfigurationDisplayName: String {
        selectedConfiguration?.displayName
            ?? (configuration.isEmpty ? "No Configuration" : configuration.suggestedName)
    }

    var activeConfigurationDisplayName: String? {
        activeConfigurationName ?? activeConfiguration?.displayName
    }

    var hasPendingSelectedConfiguration: Bool {
        guard (isRunning || telemetry.snapshot.tunnelActive), let activeConfigurationID else {
            return false
        }

        return selectedConfigurationID != activeConfigurationID
    }

    func toggleTunnel() {
        if isRunning {
            stopTunnel()
        } else {
            startTunnel()
        }
    }

    func clearLogs() {
        logManager.clearLogs()
        lastResult = "Logs cleared"
    }

    func dismissError() {
        guard state == .failed else { return }
        state = .idle
        lastResult = "Packet is idle"
    }

    func selectConfiguration(id: UUID) {
        guard let configuration = savedConfigurations.first(where: { $0.id == id }) else { return }

        selectedConfigurationID = id
        self.configuration = configuration.configuration
        persistConfigurationListState()
    }

    func addConfiguration(named name: String, configuration: TunnelConfiguration) {
        let savedConfiguration = SavedTunnelConfiguration(name: name, configuration: configuration)
        savedConfigurations.append(savedConfiguration)
        selectedConfigurationID = savedConfiguration.id
        self.configuration = configuration
        persistConfigurationListState()
    }

    func updateConfiguration(id: UUID, name: String, configuration: TunnelConfiguration) {
        guard let index = savedConfigurations.firstIndex(where: { $0.id == id }) else { return }

        savedConfigurations[index].name = name
        savedConfigurations[index].configuration = configuration

        if selectedConfigurationID == id {
            self.configuration = configuration
        }

        // Save secret to Keychain and then clear it from the configuration in savedConfigurations
        // to ensure it doesn't leak into UserDefaults.
        let secret = configuration.secret
        PacketKeychainStore.shared.save(secret: secret, for: id.uuidString)
        
        persistConfigurationListState()
    }

    func deleteConfiguration(id: UUID) {
        savedConfigurations.removeAll { $0.id == id }
        PacketKeychainStore.shared.deleteSecret(for: id.uuidString)

        if selectedConfigurationID == id {
            if let first = savedConfigurations.first {
                selectConfiguration(id: first.id)
            } else {
                selectedConfigurationID = nil
                self.configuration = TunnelConfiguration()
                persistConfigurationListState()
            }
        } else {
            persistConfigurationListState()
        }
    }

    func deleteConfigurations(at offsets: IndexSet) {
        let idsToDelete = offsets.map { savedConfigurations[$0].id }
        for id in idsToDelete {
            deleteConfiguration(id: id)
        }
    }

    func startTunnel() {
        Task {
            do {
                try validate(configuration)
                resetTelemetryForNewSession()

                let manager = try await loadOrCreateManager()
                let connectionStatus = manager.connection.status

                guard connectionStatus != .connected,
                    connectionStatus != .connecting,
                    connectionStatus != .reasserting
                else {
                    apply(connectionStatus)
                    lastResult = "Tunnel is already active"
                    return
                }

                applyConfiguration(to: manager)
                state = .launching
                lastResult = "Saving VPN configuration"
                logManager.appendLog("[APP] Saving Packet Tunnel configuration")

                try await saveToPreferences(manager)
                try await loadFromPreferences(manager)

                markConfigurationAsActive()
                providerManager = manager
                try manager.connection.startVPNTunnel()
                logManager.appendLog("[APP] Start requested through NetworkExtension")
                lastResult = "Tunnel start requested"
            } catch {
                failStart(error.localizedDescription)
            }
        }
    }

    func stopTunnel() {
        guard let providerManager else {
            lastResult = "Tunnel configuration is not loaded yet"
            return
        }

        providerManager.connection.stopVPNTunnel()
        state = .idle
        lastResult = "Tunnel stop requested"
        setTelemetryInactive(stateLabel: "idle")
        logManager.appendLog("[APP] Stop requested through NetworkExtension")
    }

    private func loadSavedConfiguration() async {
        do {
            let manager = try await loadOrCreateManager()
            providerManager = manager
            loadConfigurationState(from: manager)
            apply(manager.connection.status)
            await refreshRuntimeStats()
        } catch {
            logManager.appendLog(
                "[APP] Failed to load saved tunnel preferences: \(error.localizedDescription)")
        }
    }

    private func observeVPNStatus() {
        statusObserver = NotificationCenter.default.addObserver(
            forName: .NEVPNStatusDidChange,
            object: nil,
            queue: .main
        ) { [weak self] notification in
            guard
                let self,
                let connection = notification.object as? NEVPNConnection,
                connection == self.providerManager?.connection
            else {
                return
            }

            self.apply(connection.status)
        }
    }

    private func startStatsRefreshLoop() {
        statsRefreshTask = Task { [weak self] in
            while !Task.isCancelled {
                await self?.refreshRuntimeStats()
                try? await Task.sleep(nanoseconds: 1_000_000_000)
            }
        }
    }

    private func refreshRuntimeStats() async {
        guard
            let providerManager,
            let session = providerManager.connection as? NETunnelProviderSession
        else {
            return
        }

        let status = providerManager.connection.status
        guard status == .connected || status == .connecting || status == .reasserting else {
            return
        }

        do {
            guard let responseData = try await sendProviderMessage("stats", using: session) else {
                return
            }

            let decoder = JSONDecoder()
            decoder.keyDecodingStrategy = .convertFromSnakeCase
            let snapshot = try decoder.decode(TunnelRuntimeSnapshot.self, from: responseData)
            applyRuntimeSnapshot(snapshot)
        } catch {
            // Keep the connection alive even if telemetry polling fails temporarily.
        }
    }

    private func sendProviderMessage(
        _ command: String,
        using session: NETunnelProviderSession
    ) async throws -> Data? {
        try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Data?, Error>) in
            do {
                try session.sendProviderMessage(Data(command.utf8)) { responseData in
                    continuation.resume(returning: responseData)
                }
            } catch {
                continuation.resume(throwing: error)
            }
        }
    }

    private func applyRuntimeSnapshot(_ snapshot: TunnelRuntimeSnapshot) {
        let now = Date()
        var uploadRate = 0.0
        var downloadRate = 0.0

        if let rateAnchor,
            snapshot.bytesUp >= rateAnchor.bytesUp,
            snapshot.bytesDown >= rateAnchor.bytesDown
        {
            let elapsed = max(now.timeIntervalSince(rateAnchor.date), 0.5)
            uploadRate = Double(snapshot.bytesUp - rateAnchor.bytesUp) / elapsed
            downloadRate = Double(snapshot.bytesDown - rateAnchor.bytesDown) / elapsed
        }

        if snapshot.tunnelActive {
            self.rateAnchor = RuntimeRateAnchor(
                date: now,
                bytesUp: snapshot.bytesUp,
                bytesDown: snapshot.bytesDown
            )
        } else {
            self.rateAnchor = nil
            uploadRate = 0
            downloadRate = 0
        }

        telemetry = TunnelTelemetry(
            snapshot: snapshot,
            uploadRateBps: uploadRate,
            downloadRateBps: downloadRate
        )

        let vpnStatus = providerManager?.connection.status
        if snapshot.tunnelActive {
            state = .running
            lastResult = "Tunnel connected"
        } else if vpnStatus == .connected || vpnStatus == .connecting || vpnStatus == .reasserting {
            state = .launching
            lastResult = snapshot.lastError?.nilIfEmpty ?? "Tunnel is verifying internet access"
        }
    }

    private func resetTelemetryForNewSession() {
        rateAnchor = nil
        telemetry = .empty
    }

    private func setTelemetryInactive(stateLabel: String) {
        var snapshot = telemetry.snapshot
        snapshot.state = stateLabel
        snapshot.tunnelActive = false
        snapshot.activeStreams = 0

        rateAnchor = nil
        telemetry = TunnelTelemetry(snapshot: snapshot, uploadRateBps: 0, downloadRateBps: 0)
    }

    private func validate(_ configuration: TunnelConfiguration) throws {
        if let validationError = configuration.validationError {
            throw TunnelManagerError.invalidConfiguration(validationError)
        }

        let trimmedPort = configuration.listenPort.trimmingCharacters(in: .whitespacesAndNewlines)
        if !configuration.usesCustomCarrier,
            !trimmedPort.isEmpty,
            trimmedPort.lowercased() != "auto",
            configuration.listenPortValue == nil
        {
            throw TunnelManagerError.invalidConfiguration(
                "Listen port must be 1024-65535, or leave it blank for auto.")
        }
    }

    private func loadOrCreateManager() async throws -> NETunnelProviderManager {
        let managers = try await loadAllManagers()

        return managers.first(where: { existingManager in
            let tunnelProtocol = existingManager.protocolConfiguration as? NETunnelProviderProtocol
            let providerBundleIdentifier = tunnelProtocol?.providerBundleIdentifier
            return providerBundleIdentifier == TunnelConstants.providerBundleIdentifier
                || TunnelConstants.legacyProviderBundleIdentifiers.contains(providerBundleIdentifier ?? "")
                || TunnelConstants.legacyLocalizedDescriptions.contains(existingManager.localizedDescription ?? "")
        }) ?? NETunnelProviderManager()
    }

    private func loadConfigurationState(from manager: NETunnelProviderManager) {
        var configurations = SharedTunnelPreferenceStore.savedConfigurations
        var selectedConfigurationID = SharedTunnelPreferenceStore.selectedConfigurationID
        var activeConfigurationID = SharedTunnelPreferenceStore.activeConfigurationID
        let persistedConfiguration = persistedConfiguration(from: manager)

        if let persistedConfiguration, !persistedConfiguration.isEmpty {
            if let existingConfiguration = configurations.first(where: { $0.configuration == persistedConfiguration }) {
                activeConfigurationID = existingConfiguration.id
                activeConfigurationSnapshot = existingConfiguration.configuration
                activeConfigurationName = existingConfiguration.displayName

                if selectedConfigurationID == nil {
                    selectedConfigurationID = existingConfiguration.id
                }
            } else {
                let importedConfiguration = SavedTunnelConfiguration(
                    name: persistedConfiguration.suggestedName,
                    configuration: persistedConfiguration
                )
                configurations.insert(importedConfiguration, at: 0)
                activeConfigurationID = importedConfiguration.id
                activeConfigurationSnapshot = importedConfiguration.configuration
                activeConfigurationName = importedConfiguration.displayName

                if selectedConfigurationID == nil {
                    selectedConfigurationID = importedConfiguration.id
                }
            }
        } else {
            activeConfigurationSnapshot = nil
            activeConfigurationName = nil
        }

        if let resolvedSelectedConfigurationID = selectedConfigurationID,
            !configurations.contains(where: { $0.id == resolvedSelectedConfigurationID })
        {
            selectedConfigurationID = nil
        }

        if let resolvedActiveConfigurationID = activeConfigurationID,
            !configurations.contains(where: { $0.id == resolvedActiveConfigurationID })
        {
            activeConfigurationID = nil
        }

        // Hydrate secrets from Keychain
        for i in 0..<configurations.count {
            if let secret = PacketKeychainStore.shared.loadSecret(for: configurations[i].id.uuidString) {
                configurations[i].configuration.secret = secret
            }
        }

        configurations = refreshBuiltInProfiles(configurations)

        if selectedConfigurationID == nil {
            selectedConfigurationID = configurations.first {
                $0.name == PacketDefaultProfiles.psiphonChainName
            }?.id ?? configurations.first?.id
        }
        
        self.savedConfigurations = configurations
        self.selectedConfigurationID = selectedConfigurationID ?? configurations.first?.id
        self.activeConfigurationID = activeConfigurationID

        if let selectedConfiguration {
            configuration = selectedConfiguration.configuration
        } else if let persistedConfiguration {
            configuration = persistedConfiguration
            // Try to load secret for persisted config if possible
            if let activeId = activeConfigurationID, configuration.secret.isEmpty {
                if let secret = PacketKeychainStore.shared.loadSecret(for: activeId.uuidString) {
                    configuration.secret = secret
                }
            }
        } else {
            configuration = TunnelConfiguration()
        }

        persistConfigurationListState()
    }

    private func refreshBuiltInProfiles(
        _ configurations: [SavedTunnelConfiguration]
    ) -> [SavedTunnelConfiguration] {
        var refreshed = configurations

        func upsert(name: String, configuration: TunnelConfiguration, preferFirst: Bool = false) {
            if let index = refreshed.firstIndex(where: { $0.name == name }) {
                refreshed[index].configuration = configuration
                return
            }

            let savedConfiguration = SavedTunnelConfiguration(name: name, configuration: configuration)
            if preferFirst {
                refreshed.insert(savedConfiguration, at: 0)
            } else {
                refreshed.append(savedConfiguration)
            }
        }

        upsert(
            name: PacketDefaultProfiles.psiphonChainName,
            configuration: PacketDefaultProfiles.psiphonChainConfiguration,
            preferFirst: true
        )
        upsert(
            name: PacketDefaultProfiles.chainName,
            configuration: PacketDefaultProfiles.packetChainConfiguration
        )
        upsert(name: "Packet QUIC", configuration: PacketDefaultProfiles.quicConfiguration)

        return refreshed
    }

    private func persistedConfiguration(from manager: NETunnelProviderManager) -> TunnelConfiguration? {
        guard
            let tunnelProtocol = manager.protocolConfiguration as? NETunnelProviderProtocol,
            let providerConfiguration = tunnelProtocol.providerConfiguration
        else {
            return nil
        }

        return TunnelConfiguration(providerConfiguration: providerConfiguration)
    }

    private func markConfigurationAsActive() {
        activeConfigurationID = selectedConfigurationID
        activeConfigurationSnapshot = configuration
        activeConfigurationName = selectedConfiguration?.displayName ?? configuration.suggestedName
        
        // Ensure secret is in Keychain if it changed during selection
        if let id = activeConfigurationID {
            PacketKeychainStore.shared.save(secret: configuration.secret, for: id.uuidString)
        }
        
        persistConfigurationListState()
    }

    private func persistConfigurationListState() {
        // Strip secrets before saving to UserDefaults
        let scrubbedConfigurations = savedConfigurations.map { saved -> SavedTunnelConfiguration in
            var scrubbed = saved
            // Save to keychain before scrubbing just in case it wasn't already there
            if !scrubbed.configuration.secret.isEmpty {
                PacketKeychainStore.shared.save(secret: scrubbed.configuration.secret, for: scrubbed.id.uuidString)
            }
            scrubbed.configuration.secret = "" 
            return scrubbed
        }
        
        SharedTunnelPreferenceStore.setSavedConfigurations(scrubbedConfigurations)
        SharedTunnelPreferenceStore.setSelectedConfigurationID(selectedConfigurationID)
        SharedTunnelPreferenceStore.setActiveConfigurationID(activeConfigurationID)
    }

    private func applyConfiguration(to manager: NETunnelProviderManager) {
        let tunnelProtocol = NETunnelProviderProtocol()
        tunnelProtocol.providerBundleIdentifier = TunnelConstants.providerBundleIdentifier
        tunnelProtocol.serverAddress = configuration.remoteAddress
        tunnelProtocol.disconnectOnSleep = false
        tunnelProtocol.providerConfiguration = configuration.providerConfiguration

        manager.localizedDescription = TunnelConstants.localizedDescription
        manager.protocolConfiguration = tunnelProtocol
        manager.isEnabled = true
        manager.isOnDemandEnabled = false
    }

    private func apply(_ status: NEVPNStatus) {
        switch status {
        case .connected:
            if telemetry.snapshot.tunnelActive {
                state = .running
                lastResult = "Tunnel connected"
                return
            }

            state = .launching
            lastResult = "Tunnel is verifying internet access"

            var snapshot = telemetry.snapshot
            snapshot.state = snapshot.state.nilIfEmpty ?? "verifying"
            snapshot.tunnelActive = false
            snapshot.connectedSince = nil
            telemetry = TunnelTelemetry(
                snapshot: snapshot,
                uploadRateBps: 0,
                downloadRateBps: 0
            )
        case .connecting, .reasserting:
            state = .launching
            lastResult = "Tunnel is connecting"

            var snapshot = telemetry.snapshot
            snapshot.state = "connecting"
            telemetry = TunnelTelemetry(snapshot: snapshot, uploadRateBps: 0, downloadRateBps: 0)
        case .disconnecting:
            state = .idle
            lastResult = "Tunnel is disconnecting"
            setTelemetryInactive(stateLabel: "disconnecting")
        case .disconnected, .invalid:
            if state != .failed {
                state = .idle
            }

            if lastResult == "Not started" {
                lastResult = "Tunnel is idle"
            }

            setTelemetryInactive(stateLabel: "idle")
        @unknown default:
            state = .idle
            lastResult = "Unknown tunnel state"
            setTelemetryInactive(stateLabel: "unknown")
        }
    }

    private func failStart(_ message: String) {
        state = .failed
        lastResult = message
        setTelemetryInactive(stateLabel: "failed")
        logManager.appendLog("[APP] \(message)")
    }

    private func loadAllManagers() async throws -> [NETunnelProviderManager] {
        try await withCheckedThrowingContinuation {
            (continuation: CheckedContinuation<[NETunnelProviderManager], Error>) in
            NETunnelProviderManager.loadAllFromPreferences { managers, error in
                if let error {
                    continuation.resume(throwing: error)
                    return
                }

                continuation.resume(returning: managers ?? [])
            }
        }
    }

    private func saveToPreferences(_ manager: NETunnelProviderManager) async throws {
        try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
            manager.saveToPreferences { error in
                if let error {
                    continuation.resume(throwing: error)
                    return
                }

                continuation.resume(returning: ())
            }
        }
    }

    private func loadFromPreferences(_ manager: NETunnelProviderManager) async throws {
        try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
            manager.loadFromPreferences { error in
                if let error {
                    continuation.resume(throwing: error)
                    return
                }

                continuation.resume(returning: ())
            }
        }
    }
}

private enum TunnelManagerError: LocalizedError {
    case invalidConfiguration(String)

    var errorDescription: String? {
        switch self {
        case .invalidConfiguration(let message):
            return message
        }
    }
}
