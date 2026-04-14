import Combine
import Foundation

final class LogManager: ObservableObject {
    static let shared = LogManager()

    @Published private(set) var logs: [String] = []

    private var refreshTimer: Timer?

    private init() {
        TunnelRuntimeBridge.installLogCallback()
        logs = SharedTunnelLogStore.load()

        let timer = Timer(timeInterval: 0.5, repeats: true) { [weak self] _ in
            self?.reload()
        }
        refreshTimer = timer
        RunLoop.main.add(timer, forMode: .common)
    }

    deinit {
        refreshTimer?.invalidate()
    }

    func appendLog(_ message: String) {
        SharedTunnelLogStore.append(message)
        reload()
    }

    func clearLogs() {
        SharedTunnelLogStore.clear()
        reload()
    }

    private func reload() {
        let sharedLogs = SharedTunnelLogStore.load()
        guard sharedLogs != logs else { return }
        logs = sharedLogs
    }
}
