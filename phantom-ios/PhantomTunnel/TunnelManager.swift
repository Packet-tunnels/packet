import Combine
import Foundation
import NetworkExtension

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

    let logManager: LogManager

    private var providerManager: NETunnelProviderManager?
    private var statusObserver: NSObjectProtocol?

    init(logManager: LogManager = .shared) {
        self.logManager = logManager
        observeVPNStatus()

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

                let manager = try await loadOrCreateManager()
                let connectionStatus = manager.connection.status

                guard connectionStatus != .connected,
                      connectionStatus != .connecting,
                      connectionStatus != .reasserting else {
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
        logManager.appendLog("[APP] Stop requested through NetworkExtension")
    }

    private func loadSavedConfiguration() async {
        do {
            let manager = try await loadOrCreateManager()
            providerManager = manager
            applyPersistedConfiguration(from: manager)
            apply(manager.connection.status)
        } catch {
            logManager.appendLog("[APP] Failed to load saved tunnel preferences: \(error.localizedDescription)")
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

    private func validate(_ configuration: TunnelConfiguration) throws {
        guard !configuration.normalizedServerURL.isEmpty else {
            throw TunnelManagerError.invalidConfiguration("Server URL is required.")
        }

        guard !configuration.normalizedSecret.isEmpty else {
            throw TunnelManagerError.invalidConfiguration("Shared secret is required.")
        }

        guard let port = configuration.listenPortValue, port > 0 else {
            throw TunnelManagerError.invalidConfiguration("Listen port must be between 1 and 65535.")
        }
    }

    private func loadOrCreateManager() async throws -> NETunnelProviderManager {
        let managers = try await loadAllManagers()

        return managers.first(where: { existingManager in
            let tunnelProtocol = existingManager.protocolConfiguration as? NETunnelProviderProtocol
            return tunnelProtocol?.providerBundleIdentifier == TunnelConstants.providerBundleIdentifier
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
        case .connecting, .reasserting:
            state = .launching
            lastResult = "Tunnel is connecting"
        case .disconnecting:
            state = .idle
            lastResult = "Tunnel is disconnecting"
        case .disconnected, .invalid:
            if state != .failed {
                state = .idle
            }

            if lastResult == "Not started" {
                lastResult = "Tunnel is idle"
            }
        @unknown default:
            state = .idle
            lastResult = "Unknown tunnel state"
        }
    }

    private func failStart(_ message: String) {
        state = .failed
        lastResult = message
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
        case let .invalidConfiguration(message):
            return message
        }
    }
}
