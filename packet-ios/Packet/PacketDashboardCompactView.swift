import SwiftUI
import UIKit

struct PacketDashboardCompactView: View {
    @StateObject private var tunnelManager = TunnelManager()
    @ObservedObject private var logManager = LogManager.shared
    @State private var showSettings = false
    @State private var showCopiedToast = false
    @State private var draftConfiguration = TunnelConfiguration()
    @State private var settingsError: String?

    private let metricColumns = [
        GridItem(.flexible(), spacing: 12),
        GridItem(.flexible(), spacing: 12)
    ]

    var body: some View {
        ZStack(alignment: .bottom) {
            Color(uiColor: .systemBackground)
                .ignoresSafeArea()

            ScrollView {
                VStack(spacing: 18) {
                    headerBar
                    mainBanner
                    metricsGrid
                    logsView
                }
                .padding(.horizontal, 20)
                .padding(.top, 22)
                .padding(.bottom, 120)
            }
            .scrollDismissesKeyboard(.interactively)
            .animation(.easeInOut(duration: 0.22), value: tunnelManager.state)

            bottomBar
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .sheet(isPresented: $showSettings) {
            settingsSheet
        }
        .overlay(alignment: .top) {
            if showCopiedToast {
                copiedToast
                    .transition(.move(edge: .top).combined(with: .opacity))
            }
        }
        .animation(.easeInOut(duration: 0.2), value: showCopiedToast)
    }

    private var headerBar: some View {
        HStack {
            VStack(alignment: .leading, spacing: 4) {
                Text("Packet")
                    .font(.system(size: 20, weight: .bold))
                    .foregroundStyle(Color(uiColor: .label))

                Text("iOS VPN controller")
                    .font(.system(size: 11))
                    .foregroundStyle(.secondary)
            }

            Spacer()

            Button {
                draftConfiguration = tunnelManager.configuration
                settingsError = nil
                showSettings = true
            } label: {
                Image(systemName: "gearshape.fill")
                    .font(.system(size: 17, weight: .semibold))
                    .foregroundStyle(Color(uiColor: .label))
                    .frame(width: 34, height: 34)
                    .background(Color(uiColor: .secondarySystemBackground))
                    .clipShape(Circle())
            }
        }
    }

    private var mainBanner: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack(spacing: 10) {
                HStack(spacing: 8) {
                    Circle()
                        .fill(statusPalette.accent)
                        .frame(width: 9, height: 9)

                    Text(statusBadgeText)
                        .font(.system(size: 10, weight: .bold))
                        .foregroundStyle(statusPalette.accent)
                }
                .padding(.horizontal, 10)
                .padding(.vertical, 6)
                .background(statusPalette.badgeBackground)
                .clipShape(Capsule())

                Spacer()

                Text(statusTimerText)
                    .font(.system(size: 11, design: .monospaced))
                    .foregroundStyle(statusPalette.secondaryText)
            }

            Text(statusHeadline)
                .font(.system(size: 22, weight: .bold))
                .foregroundStyle(statusPalette.primaryText)

            Text(statusBannerText)
                .font(.system(size: 13, weight: .medium))
                .foregroundStyle(statusPalette.primaryText.opacity(0.92))
                .fixedSize(horizontal: false, vertical: true)

            Text(bannerDetailLine)
                .font(.system(size: 11, design: .monospaced))
                .foregroundStyle(statusPalette.secondaryText)
                .fixedSize(horizontal: false, vertical: true)
        }
        .padding(18)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(
            LinearGradient(
                colors: [statusPalette.backgroundTop, statusPalette.backgroundBottom],
                startPoint: .topLeading,
                endPoint: .bottomTrailing
            )
        )
        .clipShape(RoundedRectangle(cornerRadius: 22, style: .continuous))
    }

    private var metricsGrid: some View {
        let telemetry = tunnelManager.telemetry
        let snapshot = telemetry.snapshot
        let configuration = tunnelManager.configuration

        return LazyVGrid(columns: metricColumns, spacing: 12) {
            CompactMetricTile(
                title: "Transport",
                value: telemetry.transportLabel,
                detail: transportMetricDetail,
                accent: Color(red: 0.09, green: 0.39, blue: 0.87)
            )

            CompactMetricTile(
                title: "Endpoint",
                value: configuration.normalizedServerURL.isEmpty ? "Not set" : configuration.endpointHost,
                detail: configuration.normalizedServerURL.isEmpty
                    ? "Add server URL"
                    : "\(configuration.remoteAddress):\(configuration.endpointPort)",
                accent: Color(red: 0.72, green: 0.39, blue: 0.0)
            )

            CompactMetricTile(
                title: "Download",
                value: formatBytes(snapshot.bytesDown),
                detail: formatRate(telemetry.downloadRateBps),
                accent: Color(red: 0.07, green: 0.54, blue: 0.22)
            )

            CompactMetricTile(
                title: "Upload",
                value: formatBytes(snapshot.bytesUp),
                detail: formatRate(telemetry.uploadRateBps),
                accent: Color(red: 0.78, green: 0.17, blue: 0.17)
            )
        }
    }

    private var logsView: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                Text("LOGS")
                    .font(.system(size: 10, weight: .bold))
                    .foregroundStyle(.secondary)
                    .tracking(1.0)

                Spacer()

                if !logManager.logs.isEmpty {
                    Button { copyLogs() } label: {
                        Label("Copy", systemImage: "doc.on.doc")
                            .font(.system(size: 10, weight: .medium))
                            .foregroundStyle(.secondary)
                    }

                    Button(action: tunnelManager.clearLogs) {
                        Label("Clear", systemImage: "trash")
                            .font(.system(size: 10, weight: .medium))
                            .foregroundStyle(.secondary)
                    }
                    .padding(.leading, 8)
                }
            }

            if logManager.logs.isEmpty {
                Text("No logs yet. Tap Connect to start the tunnel.")
                    .font(.system(size: 12))
                    .foregroundStyle(.tertiary)
                    .frame(maxWidth: .infinity, alignment: .center)
                    .padding(.vertical, 32)
            } else {
                ScrollView {
                    VStack(alignment: .leading, spacing: 6) {
                        ForEach(Array(logManager.logs.suffix(60).enumerated()), id: \.offset) { _, log in
                            Text(log)
                                .font(.system(size: 10, design: .monospaced))
                                .foregroundStyle(Color(uiColor: .label).opacity(0.75))
                                .textSelection(.enabled)
                                .fixedSize(horizontal: false, vertical: true)
                                .frame(maxWidth: .infinity, alignment: .leading)
                        }
                    }
                }
                .frame(maxHeight: 240)
            }
        }
        .padding(16)
        .background(Color(uiColor: .secondarySystemBackground))
        .clipShape(RoundedRectangle(cornerRadius: 18, style: .continuous))
    }

    private var bottomBar: some View {
        VStack(spacing: 0) {
            Divider()

            Button(action: tunnelManager.toggleTunnel) {
                HStack(spacing: 8) {
                    if tunnelManager.isBusy {
                        ProgressView()
                            .tint(.white)
                            .scaleEffect(0.8)
                    }

                    VStack(spacing: 2) {
                        Text(tunnelManager.primaryActionTitle)
                            .font(.system(size: 16, weight: .semibold))

                        if tunnelManager.isBusy {
                            Text(tunnelManager.lastResult)
                                .font(.system(size: 10))
                                .opacity(0.85)
                                .lineLimit(1)
                        }
                    }
                }
                .frame(maxWidth: .infinity)
                .padding(.vertical, 16)
                .foregroundStyle(.white)
                .background(buttonGradient)
                .clipShape(RoundedRectangle(cornerRadius: 14, style: .continuous))
            }
            .padding(.horizontal, 20)
            .padding(.top, 12)
            .padding(.bottom, 8)
        }
        .background(Color(uiColor: .systemBackground).ignoresSafeArea(.all, edges: .bottom))
    }

    private var buttonGradient: some ShapeStyle {
        if tunnelManager.isRunning {
            return AnyShapeStyle(
                LinearGradient(
                    colors: [Color(red: 0.87, green: 0.22, blue: 0.22), Color(red: 0.72, green: 0.13, blue: 0.13)],
                    startPoint: .top,
                    endPoint: .bottom
                )
            )
        }

        return AnyShapeStyle(
            LinearGradient(
                colors: [Color(red: 0.16, green: 0.49, blue: 0.96), Color(red: 0.11, green: 0.33, blue: 0.82)],
                startPoint: .top,
                endPoint: .bottom
            )
        )
    }

    private var settingsSheet: some View {
        VStack(spacing: 0) {
            VStack(alignment: .leading, spacing: 16) {
                HStack {
                    Text("Settings")
                        .font(.system(size: 22, weight: .bold))
                        .foregroundStyle(Color(uiColor: .label))

                    Spacer()
                }
                .padding(.horizontal, 20)
                .padding(.top, 20)
                .padding(.bottom, 10)

                ScrollView {
                    VStack(spacing: 12) {
                        if let settingsError {
                            CompactSettingsErrorBanner(message: settingsError)
                        }

                        CompactSettingsTextRow(
                            label: "Server URL",
                            text: $draftConfiguration.serverURL,
                            placeholder: "https://example.com",
                            keyboard: .URL
                        )

                        CompactSettingsSecretRow(
                            label: "Shared Secret",
                            placeholder: "Enter shared secret",
                            text: $draftConfiguration.secret
                        )

                        HStack(alignment: .top, spacing: 12) {
                            CompactSettingsHalfTextField(
                                label: "Listen Port",
                                text: $draftConfiguration.listenPort,
                                placeholder: "Auto",
                                keyboard: .numberPad
                            )

                            CompactSettingsTransportRow(
                                label: "Transport",
                                selection: $draftConfiguration.transportMode
                            )
                        }

                        HStack(alignment: .top, spacing: 12) {
                            CompactSettingsHalfTextField(
                                label: "CDN Edge",
                                text: $draftConfiguration.cdnEdge,
                                placeholder: "185.143.234.235:80"
                            )

                            CompactSettingsHalfTextField(
                                label: "Host Override",
                                text: $draftConfiguration.hostOverride,
                                placeholder: "your-domain.com"
                            )
                        }

                        if let cdnEdgeValidationError = draftConfiguration.cdnEdgeValidationError {
                            CompactSettingsErrorBanner(message: cdnEdgeValidationError)
                        }
                    }
                    .padding(.horizontal, 20)
                    .padding(.bottom, 24)
                }
            }

            Divider()

            HStack(spacing: 12) {
                Button {
                    settingsError = nil
                    showSettings = false
                } label: {
                    Text("Cancel")
                        .font(.system(size: 15, weight: .semibold))
                        .frame(maxWidth: .infinity)
                        .padding(.vertical, 14)
                        .foregroundStyle(Color(red: 0.11, green: 0.33, blue: 0.82))
                        .background(Color(uiColor: .secondarySystemBackground))
                        .clipShape(RoundedRectangle(cornerRadius: 14, style: .continuous))
                }

                Button {
                    saveDraftConfiguration()
                } label: {
                    Text("Save Settings")
                        .font(.system(size: 15, weight: .semibold))
                        .frame(maxWidth: .infinity)
                        .padding(.vertical, 14)
                        .foregroundStyle(.white)
                        .background(
                            draftConfiguration.cdnEdgeValidationError == nil
                                ? Color(red: 0.11, green: 0.33, blue: 0.82)
                                : Color(uiColor: .systemGray3)
                        )
                        .clipShape(RoundedRectangle(cornerRadius: 14, style: .continuous))
                }
                .disabled(draftConfiguration.cdnEdgeValidationError != nil)
            }
            .padding(.horizontal, 20)
            .padding(.top, 14)
            .padding(.bottom, 12)
            .background(Color(uiColor: .systemBackground))
        }
        .background(Color(uiColor: .systemBackground))
        .presentationDragIndicator(.visible)
        .onAppear {
            draftConfiguration = tunnelManager.configuration
            settingsError = nil
        }
    }

    private var copiedToast: some View {
        HStack(spacing: 6) {
            Image(systemName: "checkmark.circle.fill")
                .foregroundStyle(.green)
                .font(.system(size: 13))

            Text("Logs copied to clipboard")
                .font(.system(size: 12, weight: .medium))
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 10)
        .background(.ultraThinMaterial)
        .clipShape(Capsule())
        .shadow(color: .black.opacity(0.1), radius: 8, y: 4)
        .padding(.top, 60)
    }

    private var statusBadgeText: String {
        switch tunnelManager.state {
        case .running:
            return "ACTIVE"
        case .launching:
            return "STARTING"
        case .failed:
            return "ERROR"
        case .idle:
            return "IDLE"
        }
    }

    private var statusHeadline: String {
        switch tunnelManager.state {
        case .running:
            return "Tunnel is active"
        case .launching:
            return "Starting tunnel"
        case .failed:
            return "Tunnel needs attention"
        case .idle:
            return "Tunnel is idle"
        }
    }

    private var statusTimerText: String {
        let snapshot = tunnelManager.telemetry.snapshot

        if (tunnelManager.state == .running || snapshot.tunnelActive),
            let connectedSince = snapshot.connectedSince
        {
            return "\(formatConnectedDuration(connectedSince)) live"
        }

        switch tunnelManager.state {
        case .launching:
            return "Bringing tunnel online"
        case .failed:
            return "Check settings"
        case .idle:
            return "Not connected"
        case .running:
            return "Not connected"
        }
    }

    private var statusBannerText: String {
        let snapshot = tunnelManager.telemetry.snapshot
        let configuration = tunnelManager.configuration

        if let lastError = snapshot.lastError?.nilIfEmpty {
            return lastError
        }

        if let cdnEdgeValidationError = configuration.cdnEdgeValidationError {
            return cdnEdgeValidationError
        }

        if tunnelManager.state == .running || snapshot.tunnelActive {
            let port = snapshot.listenPort.map { String($0) }
                ?? configuration.listenPortValue.map { String($0) }
                ?? configuration.listenPort.trimmingCharacters(in: .whitespacesAndNewlines).nilIfEmpty
                ?? "auto"

            if snapshot.activeStreams > 0 {
                return "Forwarding is active on 127.0.0.1:\(port) with \(snapshot.activeStreams) live stream(s)."
            }

            return "Forwarding is active on 127.0.0.1:\(port). Waiting for device traffic."
        }

        if configuration.normalizedServerURL.isEmpty || configuration.normalizedSecret.isEmpty {
            return "Add your server URL and shared secret before connecting."
        }

        switch tunnelManager.state {
        case .launching:
            return "Saving the NetworkExtension profile and starting the packet tunnel."
        case .failed:
            return tunnelManager.lastResult
        case .idle:
            return "Connection is ready. Tap Connect Tunnel when you want to go live."
        case .running:
            return tunnelManager.lastResult
        }
    }

    private var bannerDetailLine: String {
        let snapshot = tunnelManager.telemetry.snapshot
        let configuration = tunnelManager.configuration
        let parts = [
            configuration.ingressLabel,
            configuration.normalizedServerURL.isEmpty
                ? "No endpoint configured"
                : "\(configuration.endpointHost):\(configuration.endpointPort)",
            snapshot.lastPingMs.map { "\($0) ms ping" } ?? "\(snapshot.activeStreams) stream(s)"
        ]

        return parts.joined(separator: " · ")
    }

    private var transportMetricDetail: String {
        let snapshot = tunnelManager.telemetry.snapshot
        let configuration = tunnelManager.configuration
        let port = snapshot.listenPort.map { String($0) }
            ?? configuration.listenPortValue.map { String($0) }
            ?? configuration.listenPort.trimmingCharacters(in: .whitespacesAndNewlines).nilIfEmpty
            ?? "Auto"

        return "\(snapshot.state.nilIfEmpty ?? tunnelManager.state.rawValue) · Port \(port)"
    }

    private var statusPalette: CompactBannerPalette {
        switch tunnelManager.state {
        case .running:
            return CompactBannerPalette(
                backgroundTop: Color(red: 0.91, green: 0.98, blue: 0.93),
                backgroundBottom: Color(red: 0.84, green: 0.96, blue: 0.88),
                badgeBackground: Color(red: 0.83, green: 0.95, blue: 0.86),
                accent: Color(red: 0.07, green: 0.54, blue: 0.22),
                primaryText: Color(red: 0.08, green: 0.28, blue: 0.14),
                secondaryText: Color(red: 0.16, green: 0.42, blue: 0.22)
            )
        case .launching:
            return CompactBannerPalette(
                backgroundTop: Color(red: 1.0, green: 0.96, blue: 0.89),
                backgroundBottom: Color(red: 1.0, green: 0.92, blue: 0.8),
                badgeBackground: Color(red: 0.99, green: 0.9, blue: 0.73),
                accent: Color(red: 0.72, green: 0.39, blue: 0.0),
                primaryText: Color(red: 0.43, green: 0.23, blue: 0.01),
                secondaryText: Color(red: 0.58, green: 0.31, blue: 0.03)
            )
        case .failed:
            return CompactBannerPalette(
                backgroundTop: Color(red: 1.0, green: 0.93, blue: 0.93),
                backgroundBottom: Color(red: 0.99, green: 0.88, blue: 0.88),
                badgeBackground: Color(red: 0.98, green: 0.85, blue: 0.85),
                accent: Color(red: 0.78, green: 0.17, blue: 0.17),
                primaryText: Color(red: 0.43, green: 0.08, blue: 0.08),
                secondaryText: Color(red: 0.58, green: 0.13, blue: 0.13)
            )
        case .idle:
            return CompactBannerPalette(
                backgroundTop: Color(red: 0.95, green: 0.97, blue: 1.0),
                backgroundBottom: Color(red: 0.91, green: 0.94, blue: 1.0),
                badgeBackground: Color(red: 0.88, green: 0.92, blue: 0.99),
                accent: Color(red: 0.16, green: 0.37, blue: 0.82),
                primaryText: Color(red: 0.1, green: 0.19, blue: 0.39),
                secondaryText: Color(red: 0.18, green: 0.28, blue: 0.52)
            )
        }
    }

    private func copyLogs() {
        UIPasteboard.general.string = logManager.logs.joined(separator: "\n")
        showCopiedToast = true

        let feedback = UIImpactFeedbackGenerator(style: .light)
        feedback.impactOccurred()

        DispatchQueue.main.asyncAfter(deadline: .now() + 1.5) {
            showCopiedToast = false
        }
    }

    private func saveDraftConfiguration() {
        if let cdnEdgeValidationError = draftConfiguration.cdnEdgeValidationError {
            settingsError = cdnEdgeValidationError
            return
        }

        tunnelManager.configuration = draftConfiguration
        settingsError = nil
        showSettings = false
    }

    private func formatBytes(_ bytes: UInt64) -> String {
        ByteCountFormatter.string(fromByteCount: Int64(bytes), countStyle: .binary)
    }

    private func formatRate(_ bytesPerSecond: Double) -> String {
        "\(formatBytes(UInt64(max(bytesPerSecond, 0))))/s"
    }

    private func formatConnectedDuration(_ connectedSinceSeconds: UInt64) -> String {
        let nowSeconds = UInt64(Date().timeIntervalSince1970)
        let elapsedSeconds = max(Int64(nowSeconds) - Int64(connectedSinceSeconds), 0)
        let hours = elapsedSeconds / 3600
        let minutes = (elapsedSeconds % 3600) / 60
        let seconds = elapsedSeconds % 60

        if hours > 0 {
            return String(format: "%02lld:%02lld:%02lld", hours, minutes, seconds)
        }

        return String(format: "%02lld:%02lld", minutes, seconds)
    }
}

private struct CompactMetricTile: View {
    let title: String
    let value: String
    let detail: String
    let accent: Color

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(title.uppercased())
                .font(.system(size: 9, weight: .bold))
                .foregroundStyle(accent.opacity(0.9))
                .tracking(0.8)

            Text(value)
                .font(.system(size: 15, weight: .bold))
                .foregroundStyle(Color(uiColor: .label))
                .lineLimit(2)
                .minimumScaleFactor(0.82)
                .fixedSize(horizontal: false, vertical: true)

            Text(detail)
                .font(.system(size: 10))
                .foregroundStyle(Color(uiColor: .secondaryLabel))
                .fixedSize(horizontal: false, vertical: true)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(14)
        .background(Color(uiColor: .secondarySystemBackground))
        .clipShape(RoundedRectangle(cornerRadius: 18, style: .continuous))
    }
}

private struct CompactSettingsTextRow: View {
    let label: String
    @Binding var text: String
    let placeholder: String
    var keyboard: UIKeyboardType = .default

    var body: some View {
        HStack(spacing: 14) {
            Text(label)
                .font(.system(size: 13, weight: .semibold))
                .foregroundStyle(Color(uiColor: .label))
                .frame(width: 104, alignment: .leading)

            TextField(placeholder, text: $text)
                .font(.system(size: 14, design: .monospaced))
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .keyboardType(keyboard)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
        .padding(14)
        .background(Color(uiColor: .secondarySystemBackground))
        .clipShape(RoundedRectangle(cornerRadius: 16, style: .continuous))
    }
}

private struct CompactSettingsSecretRow: View {
    let label: String
    let placeholder: String
    @Binding var text: String
    @State private var isVisible = false

    var body: some View {
        HStack(spacing: 14) {
            Text(label)
                .font(.system(size: 13, weight: .semibold))
                .foregroundStyle(Color(uiColor: .label))
                .frame(width: 104, alignment: .leading)

            Group {
                if isVisible {
                    TextField(placeholder, text: $text)
                } else {
                    SecureField(placeholder, text: $text)
                }
            }
            .font(.system(size: 14, design: .monospaced))
            .textInputAutocapitalization(.never)
            .autocorrectionDisabled()

            Button {
                isVisible.toggle()
            } label: {
                Image(systemName: isVisible ? "eye.slash" : "eye")
                    .foregroundStyle(.secondary)
            }
            .buttonStyle(.borderless)

            if !text.isEmpty {
                Button {
                    text = ""
                } label: {
                    Image(systemName: "xmark.circle.fill")
                        .foregroundStyle(.secondary)
                }
                .buttonStyle(.borderless)
            }

            Button {
                if let pasted = UIPasteboard.general.string {
                    text = pasted
                }
            } label: {
                Image(systemName: "doc.on.clipboard")
                    .foregroundStyle(.secondary)
            }
            .buttonStyle(.borderless)
        }
        .padding(14)
        .background(Color(uiColor: .secondarySystemBackground))
        .clipShape(RoundedRectangle(cornerRadius: 16, style: .continuous))
    }
}

private struct CompactSettingsTransportRow: View {
    let label: String
    @Binding var selection: TunnelTransportMode

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(label)
                .font(.system(size: 13, weight: .semibold))
                .foregroundStyle(Color(uiColor: .label))
                .frame(maxWidth: .infinity, alignment: .leading)

            Menu {
                ForEach(TunnelTransportMode.allCases) { mode in
                    Button(mode.title) {
                        selection = mode
                    }
                }
            } label: {
                HStack(spacing: 8) {
                    Text(selection.title)
                        .font(.system(size: 14, weight: .semibold))

                    Image(systemName: "chevron.down")
                        .font(.system(size: 11, weight: .bold))
                }
                .foregroundStyle(Color(uiColor: .label))
                .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
        .padding(14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Color(uiColor: .secondarySystemBackground))
        .clipShape(RoundedRectangle(cornerRadius: 16, style: .continuous))
    }
}

private struct CompactSettingsHalfTextField: View {
    let label: String
    @Binding var text: String
    let placeholder: String
    var keyboard: UIKeyboardType = .default

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(label)
                .font(.system(size: 13, weight: .semibold))
                .foregroundStyle(Color(uiColor: .label))

            TextField(placeholder, text: $text)
                .font(.system(size: 14, design: .monospaced))
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .keyboardType(keyboard)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
        .padding(14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Color(uiColor: .secondarySystemBackground))
        .clipShape(RoundedRectangle(cornerRadius: 16, style: .continuous))
    }
}

private struct CompactSettingsErrorBanner: View {
    let message: String

    var body: some View {
        HStack(spacing: 8) {
            Image(systemName: "exclamationmark.triangle.fill")
                .foregroundStyle(.red)

            Text(message)
                .font(.system(size: 12))
                .foregroundStyle(Color(uiColor: .label))
                .fixedSize(horizontal: false, vertical: true)

            Spacer(minLength: 0)
        }
        .padding(12)
        .background(Color.red.opacity(0.08))
        .clipShape(RoundedRectangle(cornerRadius: 14, style: .continuous))
    }
}

private struct CompactBannerPalette {
    let backgroundTop: Color
    let backgroundBottom: Color
    let badgeBackground: Color
    let accent: Color
    let primaryText: Color
    let secondaryText: Color
}

private extension String {
    var nilIfEmpty: String? {
        let trimmed = trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? nil : trimmed
    }
}
