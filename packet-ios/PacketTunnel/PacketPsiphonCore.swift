import Foundation

#if canImport(PsiphonTunnel)
import PsiphonTunnel
#endif

struct PacketPsiphonStartResult {
    let httpPort: UInt16
    let socksPort: UInt16?
}

#if canImport(PsiphonTunnel)
final class PacketPsiphonCore: NSObject, TunneledAppDelegate {
    static let shared = PacketPsiphonCore()

    private let stateLock = NSLock()
    private var tunnel: PsiphonTunnel?
    private var configJSON = ""
    private var httpPort: UInt16?
    private var socksPort: UInt16?
    private var connected = false
    private var lastError: String?

    func startLocalProxy(
        upstreamProxyURL: String,
        requestedHTTPPort: Int,
        requestedSocksPort: Int
    ) async throws -> PacketPsiphonStartResult {
        stop()

        configJSON = try makeClientConfig(
            upstreamProxyURL: upstreamProxyURL,
            requestedHTTPPort: requestedHTTPPort,
            requestedSocksPort: requestedSocksPort
        )
        httpPort = nil
        socksPort = nil
        connected = false
        lastError = nil

        let tunnel = PsiphonTunnel.newPsiphonTunnel(self)
        self.tunnel = tunnel

        SharedTunnelLogStore.append("[PSIPHON] Starting iOS Psiphon core through \(redactedProxy(upstreamProxyURL))")
        guard tunnel.start(false) else {
            throw PacketPsiphonCoreError.startFailed("PsiphonTunnel.start(false) returned false.")
        }

        let deadline = Date().addingTimeInterval(30)
        while Date() < deadline {
            if let httpPort = currentHTTPPort() {
                return PacketPsiphonStartResult(httpPort: httpPort, socksPort: currentSocksPort())
            }

            if let lastError = currentLastError() {
                SharedTunnelLogStore.append("[PSIPHON] Waiting after upstream error: \(lastError)")
            }

            try await Task.sleep(nanoseconds: 200_000_000)
        }

        throw PacketPsiphonCoreError.startTimeout
    }

    func stop() {
        tunnel?.stop()
        tunnel = nil
        stateLock.lock()
        httpPort = nil
        socksPort = nil
        connected = false
        lastError = nil
        stateLock.unlock()
    }

    func getPsiphonConfig() -> Any? {
        configJSON
    }

    func getEmbeddedServerEntries() -> String? {
        ""
    }

    func onConnecting() {
        SharedTunnelLogStore.append("[PSIPHON] Connecting")
    }

    func onConnected() {
        stateLock.lock()
        connected = true
        stateLock.unlock()
        SharedTunnelLogStore.append("[PSIPHON] Connected")
    }

    func onExiting() {
        SharedTunnelLogStore.append("[PSIPHON] Exiting")
    }

    func onDiagnosticMessage(_ message: String, withTimestamp timestamp: String) {
        SharedTunnelLogStore.append("[PSIPHON] \(message)")
    }

    func onListeningHttpProxyPort(_ port: Int) {
        guard let port = UInt16(exactly: port), port > 0 else { return }
        stateLock.lock()
        httpPort = port
        stateLock.unlock()
        SharedTunnelLogStore.append("[PSIPHON] Local HTTP proxy listening on 127.0.0.1:\(port)")
    }

    func onListeningSocksProxyPort(_ port: Int) {
        guard let port = UInt16(exactly: port), port > 0 else { return }
        stateLock.lock()
        socksPort = port
        stateLock.unlock()
        SharedTunnelLogStore.append("[PSIPHON] Local SOCKS proxy listening on 127.0.0.1:\(port)")
    }

    func onUpstreamProxyError(_ message: String) {
        stateLock.lock()
        lastError = message
        stateLock.unlock()
        SharedTunnelLogStore.append("[PSIPHON] Upstream proxy error: \(message)")
    }

    private func makeClientConfig(
        upstreamProxyURL: String,
        requestedHTTPPort: Int,
        requestedSocksPort: Int
    ) throws -> String {
        let bundledURL = Bundle.main.url(
            forResource: "client",
            withExtension: "config",
            subdirectory: "psiphon"
        ) ?? Bundle.main.url(forResource: "client", withExtension: "config")

        guard let url = bundledURL else {
            throw PacketPsiphonCoreError.missingClientConfig
        }

        let data = try Data(contentsOf: url)
        guard var object = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            throw PacketPsiphonCoreError.invalidClientConfig("client.config is not a JSON object.")
        }

        object["UpstreamProxyURL"] = upstreamProxyURL
        object["UpstreamProxyUrl"] = upstreamProxyURL
        object["LocalHttpProxyPort"] = requestedHTTPPort
        object["LocalSocksProxyPort"] = requestedSocksPort
        object["EmitDiagnosticNotices"] = true
        object["EmitBytesTransferred"] = true
        object["EstablishTunnelTimeoutSeconds"] = 0

        let updatedData = try JSONSerialization.data(withJSONObject: object)
        guard let updated = String(data: updatedData, encoding: .utf8) else {
            throw PacketPsiphonCoreError.invalidClientConfig("client.config could not be encoded as UTF-8.")
        }

        return updated
    }

    private func currentHTTPPort() -> UInt16? {
        stateLock.lock()
        defer { stateLock.unlock() }
        return httpPort
    }

    private func currentSocksPort() -> UInt16? {
        stateLock.lock()
        defer { stateLock.unlock() }
        return socksPort
    }

    private func currentLastError() -> String? {
        stateLock.lock()
        defer { stateLock.unlock() }
        return lastError
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
}
#else
final class PacketPsiphonCore {
    static let shared = PacketPsiphonCore()

    func startLocalProxy(
        upstreamProxyURL: String,
        requestedHTTPPort: Int,
        requestedSocksPort: Int
    ) async throws -> PacketPsiphonStartResult {
        throw PacketPsiphonCoreError.engineMissing
    }

    func stop() {}
}
#endif

private enum PacketPsiphonCoreError: LocalizedError {
    case engineMissing
    case missingClientConfig
    case invalidClientConfig(String)
    case startFailed(String)
    case startTimeout

    var errorDescription: String? {
        switch self {
        case .engineMissing:
            return "Psiphon iOS engine is not linked. Run packet-ios/scripts/install-psiphon-ios-framework.sh and regenerate the Xcode project."
        case .missingClientConfig:
            return "Missing PacketTunnel/Resources/psiphon/client.config. Generate it on the Psiphon server and run packet-android/scripts/psiphon-core-lab.sh install-client-asset."
        case let .invalidClientConfig(message):
            return message
        case let .startFailed(message):
            return message
        case .startTimeout:
            return "Psiphon core started but did not expose a local HTTP proxy within 30s."
        }
    }
}
