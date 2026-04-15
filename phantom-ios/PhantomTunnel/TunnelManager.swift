import Combine
import Foundation
import NetworkExtension

struct TunnelTelemetry: Equatable {
    var snapshot: TunnelRuntimeSnapshot = .empty
    var uploadRateBps: Double = 0
    var downloadRateBps: Double = 0

    static let empty = TunnelTelemetry()

    var countryLabel: String {
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
    @Published private(set) var state: State = .idle
    @Published private(set) var lastResult = "Not started"
    @Published private(set) var telemetry = TunnelTelemetry.empty

    let logManager: LogManager

    private var providerManager: NETunnelProviderManager?
    private var statusObserver: NSObjectProtocol?
    private var statsRefreshTask: Task<Void, Never>?
    private var rateAnchor: RuntimeRateAnchor?

    init(logManager: LogManager = .shared) {
        self.logManager = logManager
        observeVPNStatus()
        startStatsRefreshLoop()

        if logManager.logs.isEmpty {
            logManager.appendLog("[APP] Phantom Tunnel UI is ready")
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
            applyPersistedConfiguration(from: manager)
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
        try await withCheckedThrowingContinuation { continuation in
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
        guard !configuration.normalizedServerURL.isEmpty else {
            throw TunnelManagerError.invalidConfiguration("Server URL is required.")
        }

        guard !configuration.normalizedSecret.isEmpty else {
            throw TunnelManagerError.invalidConfiguration("Shared secret is required.")
        }

        let trimmedPort = configuration.listenPort.trimmingCharacters(in: .whitespacesAndNewlines)
        if !trimmedPort.isEmpty,
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
            return tunnelProtocol?.providerBundleIdentifier
                == TunnelConstants.providerBundleIdentifier
        }) ?? NETunnelProviderManager()
    }

    private func applyPersistedConfiguration(from manager: NETunnelProviderManager) {
        guard
            let tunnelProtocol = manager.protocolConfiguration as? NETunnelProviderProtocol,
            let providerConfiguration = tunnelProtocol.providerConfiguration
        else {
            return
        }

        configuration = TunnelConfiguration(providerConfiguration: providerConfiguration)
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
            state = .running
            lastResult = "Tunnel connected"

            var snapshot = telemetry.snapshot
            snapshot.state = "connected"
            snapshot.tunnelActive = true
            telemetry = TunnelTelemetry(
                snapshot: snapshot,
                uploadRateBps: telemetry.uploadRateBps,
                downloadRateBps: telemetry.downloadRateBps
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
        try await withCheckedThrowingContinuation { continuation in
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
        try await withCheckedThrowingContinuation { continuation in
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
        try await withCheckedThrowingContinuation { continuation in
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
