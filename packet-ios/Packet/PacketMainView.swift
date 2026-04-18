import SwiftUI
import UIKit

struct PacketMainView: View {
    @ObservedObject var tunnelManager: TunnelManager
    @ObservedObject var complianceStore: PacketComplianceStore

    @State private var showingDisclosureSheet = false
    @State private var shouldStartTunnelAfterDisclosure = false
    @State private var hasPresentedInitialDisclosure = false

    private let metricColumns = [
        GridItem(.flexible(), spacing: 16),
        GridItem(.flexible(), spacing: 16)
    ]

    var body: some View {
        NavigationStack {
            ScrollView(.vertical, showsIndicators: false) {
                VStack(spacing: 24) {
                    if !complianceStore.vpnDisclosureAcknowledged {
                        disclosureReminderCard
                    }

                    mainDashboardCard
                    metricsGrid
                }
                .padding(.horizontal, 24)
                .padding(.top, 16)
                .padding(.bottom, 40)
            }
            .background(Color(uiColor: .systemGroupedBackground).ignoresSafeArea())
            .scrollDismissesKeyboard(.interactively)
            .animation(.spring(response: 0.4, dampingFraction: 0.8), value: tunnelManager.state)
            .navigationTitle("Packet")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    HStack(spacing: 8) {
                        Circle()
                            .fill(statusColor)
                            .frame(width: 6, height: 6)
                            .shadow(color: statusColor.opacity(0.6), radius: tunnelManager.isRunning ? 4 : 0)
                            .animation(
                                tunnelManager.isRunning
                                    ? .easeInOut(duration: 1).repeatForever()
                                    : .default,
                                value: tunnelManager.isRunning
                            )
                    }
                }
            }
            .sheet(isPresented: $showingDisclosureSheet, onDismiss: handleDisclosureDismissed) {
                PacketVPNDisclosureSheet(
                    isConnectFlow: shouldStartTunnelAfterDisclosure,
                    acceptTitle: shouldStartTunnelAfterDisclosure ? "Accept & Connect" : "Accept",
                    onAccept: acceptDisclosure,
                    onDismiss: dismissDisclosure
                )
            }
            .onAppear(perform: presentInitialDisclosureIfNeeded)
        }
    }

    // MARK: - Views

    private var mainDashboardCard: some View {
        VStack(alignment: .leading, spacing: 20) {
            HStack(alignment: .top) {
                VStack(alignment: .leading, spacing: 6) {
                    Text(statusHeadline.uppercased())
                        .font(.system(size: 12, weight: .bold, design: .monospaced))
                        .foregroundStyle(statusColor)
                        .tracking(1.0)

                    Text(tunnelManager.state == .running ? "Secured" : "Offline")
                        .font(.system(size: 32, weight: .semibold, design: .rounded))
                        .foregroundStyle(.primary)
                }

                Spacer()

                if tunnelManager.isRunning || tunnelManager.telemetry.snapshot.tunnelActive {
                    Text(statusTimerText)
                        .font(.system(size: 14, weight: .medium, design: .monospaced))
                        .foregroundStyle(.secondary)
                        .padding(.horizontal, 12)
                        .padding(.vertical, 6)
                        .background(Color(uiColor: .tertiarySystemFill))
                        .clipShape(Capsule())
                }
            }

            Text(statusBannerText)
                .font(.system(size: 14, weight: .regular))
                .foregroundStyle(.secondary)
                .lineLimit(2)
                .fixedSize(horizontal: false, vertical: true)

            Divider()
                .overlay(Color(uiColor: .quaternarySystemFill))

            HStack {
                Text(bannerDetailLine)
                    .font(.system(size: 11, weight: .medium, design: .monospaced))
                    .foregroundStyle(.tertiary)
                    .lineLimit(1)

                Spacer()

                actionButton
            }
        }
        .padding(24)
        .background(Color(uiColor: .secondarySystemGroupedBackground))
        .clipShape(RoundedRectangle(cornerRadius: 24, style: .continuous))
        .shadow(color: Color.black.opacity(0.04), radius: 16, x: 0, y: 8)
    }

    private var disclosureReminderCard: some View {
        HStack(alignment: .top, spacing: 16) {
            Image(systemName: "exclamationmark.shield.fill")
                .foregroundStyle(.orange)
                .font(.system(size: 20))

            VStack(alignment: .leading, spacing: 4) {
                Text("Action Required")
                    .font(.system(size: 14, weight: .semibold))
                    .foregroundStyle(.primary)

                Text("Accept the in-app VPN disclosure before initiating your first connection.")
                    .font(.system(size: 13))
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .padding(16)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Color.orange.opacity(0.1))
        .clipShape(RoundedRectangle(cornerRadius: 16, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: 16, style: .continuous)
                .stroke(Color.orange.opacity(0.3), lineWidth: 1)
        )
    }

    private var metricsGrid: some View {
        let telemetry = tunnelManager.telemetry
        let snapshot = telemetry.snapshot
        let configuration = tunnelManager.configuration

        return LazyVGrid(columns: metricColumns, spacing: 16) {
            ModernMetricTile(
                icon: "network",
                title: "Transport",
                value: telemetry.transportLabel,
                detail: transportMetricDetail
            )

            ModernMetricTile(
                icon: "server.rack",
                title: "Endpoint",
                value: configuration.normalizedServerURL.isEmpty
                    ? "Not configured"
                    : configuration.endpointHost,
                detail: configuration.normalizedServerURL.isEmpty
                    ? "Missing URL"
                    : "\(configuration.remoteAddress)"
            )

            ModernMetricTile(
                icon: "arrow.down.right",
                title: "Inbound",
                value: formatBytes(snapshot.bytesDown),
                detail: formatRate(telemetry.downloadRateBps)
            )

            ModernMetricTile(
                icon: "arrow.up.left",
                title: "Outbound",
                value: formatBytes(snapshot.bytesUp),
                detail: formatRate(telemetry.uploadRateBps)
            )
        }
    }

    private var actionButton: some View {
        Button(action: handlePrimaryAction) {
            HStack(spacing: 6) {
                if tunnelManager.isBusy {
                    ProgressView()
                        .tint(tunnelManager.isRunning ? .white : .primary)
                        .controlSize(.small)
                } else {
                    Image(systemName: tunnelManager.isRunning ? "stop.fill" : "play.fill")
                        .font(.system(size: 10, weight: .bold))
                }

                Text(primaryActionTitle)
                    .font(.system(size: 13, weight: .bold, design: .rounded))
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 10)
            .background(tunnelManager.isRunning ? Color.red : Color.primary)
            .foregroundStyle(tunnelManager.isRunning ? .white : Color(uiColor: .systemBackground))
            .clipShape(Capsule())
        }
        .buttonStyle(.plain)
        .disabled(tunnelManager.isBusy)
    }

    // MARK: - Properties & Logic

    private var statusColor: Color {
        switch tunnelManager.state {
        case .running:
            return .green
        case .launching:
            return .orange
        case .failed:
            return .red
        case .idle:
            return .gray
        }
    }

    private var statusHeadline: String {
        switch tunnelManager.state {
        case .running:
            return "Active Connection"
        case .launching:
            return "Initializing"
        case .failed:
            return "Connection Failed"
        case .idle:
            return "Standby Mode"
        }
    }

    private var statusTimerText: String {
        let snapshot = tunnelManager.telemetry.snapshot
        if (tunnelManager.state == .running || snapshot.tunnelActive),
            let connectedSince = snapshot.connectedSince
        {
            return formatConnectedDuration(connectedSince)
        }
        return "00:00"
    }

    private var statusBannerText: String {
        let snapshot = tunnelManager.telemetry.snapshot
        let configuration = tunnelManager.configuration

        if !complianceStore.vpnDisclosureAcknowledged {
            return "Accept the VPN data-use disclosure to enable the first connection."
        }
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

            return snapshot.activeStreams > 0
                ? "Forwarding active on localhost:\(port) with \(snapshot.activeStreams) stream(s)."
                : "Forwarding active on localhost:\(port). Awaiting traffic."
        }

        if configuration.normalizedServerURL.isEmpty || configuration.normalizedSecret.isEmpty {
            return "Configuration incomplete. Set URL and secret in Settings."
        }

        switch tunnelManager.state {
        case .launching:
            return "Applying network profile and initiating tunnel."
        case .failed:
            return tunnelManager.lastResult
        case .idle:
            return "System ready. Initiate connection to begin routing."
        case .running:
            return tunnelManager.lastResult
        }
    }

    private var bannerDetailLine: String {
        let snapshot = tunnelManager.telemetry.snapshot
        let configuration = tunnelManager.configuration
        let parts = [
            configuration.ingressLabel,
            snapshot.lastPingMs.map { "\($0)ms" } ?? "0ms"
        ]
        return parts.joined(separator: " • ")
    }

    private var transportMetricDetail: String {
        let snapshot = tunnelManager.telemetry.snapshot
        let configuration = tunnelManager.configuration
        let port = snapshot.listenPort.map { String($0) }
            ?? configuration.listenPortValue.map { String($0) }
            ?? configuration.listenPort.trimmingCharacters(in: .whitespacesAndNewlines).nilIfEmpty
            ?? "Auto"

        return "\(snapshot.state.nilIfEmpty ?? tunnelManager.state.rawValue.capitalized) • Port \(port)"
    }

    private var primaryActionTitle: String {
        tunnelManager.isRunning ? "Stop" : "Connect"
    }

    private func handlePrimaryAction() {
        let feedback = UIImpactFeedbackGenerator(style: .medium)
        feedback.impactOccurred()

        guard tunnelManager.isRunning || complianceStore.vpnDisclosureAcknowledged else {
            shouldStartTunnelAfterDisclosure = true
            showingDisclosureSheet = true
            return
        }

        tunnelManager.toggleTunnel()
    }

    private func presentInitialDisclosureIfNeeded() {
        guard !hasPresentedInitialDisclosure else { return }
        hasPresentedInitialDisclosure = true

        guard !complianceStore.vpnDisclosureAcknowledged else { return }
        showingDisclosureSheet = true
    }

    private func acceptDisclosure() {
        complianceStore.setVPNDisclosureAcknowledged(true)

        let shouldStartTunnel = shouldStartTunnelAfterDisclosure && !tunnelManager.isRunning
        shouldStartTunnelAfterDisclosure = false
        showingDisclosureSheet = false

        guard shouldStartTunnel else { return }
        tunnelManager.toggleTunnel()
    }

    private func dismissDisclosure() {
        shouldStartTunnelAfterDisclosure = false
        showingDisclosureSheet = false
    }

    private func handleDisclosureDismissed() {
        shouldStartTunnelAfterDisclosure = false
    }

    // MARK: - Formatters

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

// MARK: - Subviews

struct ModernMetricTile: View {
    let icon: String
    let title: String
    let value: String
    let detail: String

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                Image(systemName: icon)
                    .font(.system(size: 14, weight: .medium))
                    .foregroundStyle(.secondary)

                Text(title.uppercased())
                    .font(.system(size: 10, weight: .bold, design: .monospaced))
                    .foregroundStyle(.tertiary)

                Spacer()
            }

            VStack(alignment: .leading, spacing: 2) {
                Text(value)
                    .font(.system(size: 18, weight: .semibold, design: .rounded))
                    .foregroundStyle(.primary)
                    .lineLimit(1)
                    .minimumScaleFactor(0.8)

                Text(detail)
                    .font(.system(size: 12, weight: .medium, design: .monospaced))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
        }
        .padding(16)
        .background(Color(uiColor: .secondarySystemGroupedBackground))
        .clipShape(RoundedRectangle(cornerRadius: 20, style: .continuous))
        .shadow(color: Color.black.opacity(0.02), radius: 10, x: 0, y: 4)
    }
}

private struct PacketVPNDisclosureSheet: View {
    let isConnectFlow: Bool
    let acceptTitle: String
    let onAccept: () -> Void
    let onDismiss: () -> Void

    var body: some View {
        NavigationStack {
            VStack(alignment: .leading, spacing: 20) {
                VStack(alignment: .leading, spacing: 12) {
                    Label("VPN Data-Use Disclosure", systemImage: "checkmark.shield.fill")
                        .font(.system(size: 18, weight: .semibold))
                        .foregroundStyle(.primary)

                    Text("Packet creates an iOS VPN profile and routes your traffic through the configured tunnel while connected.")
                        .font(.system(size: 14))
                        .foregroundStyle(.secondary)

                    Text("Your server settings and disclosure acknowledgement are stored locally on this device so the tunnel can reconnect with the same configuration.")
                        .font(.system(size: 14))
                        .foregroundStyle(.secondary)

                    Text("You can disconnect at any time from Packet or from the system VPN controls in iOS Settings.")
                        .font(.system(size: 14))
                        .foregroundStyle(.secondary)
                }

                Spacer(minLength: 0)

                VStack(spacing: 12) {
                    Button(action: onAccept) {
                        Text(acceptTitle)
                            .font(.system(size: 16, weight: .semibold, design: .rounded))
                            .frame(maxWidth: .infinity)
                            .padding(.vertical, 14)
                            .background(Color.primary)
                            .foregroundStyle(Color(uiColor: .systemBackground))
                            .clipShape(RoundedRectangle(cornerRadius: 16, style: .continuous))
                    }

                    Button(isConnectFlow ? "Not Now" : "Dismiss", action: onDismiss)
                        .font(.system(size: 15, weight: .medium))
                        .foregroundStyle(.secondary)
                }
            }
            .padding(24)
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            .background(Color(uiColor: .systemGroupedBackground).ignoresSafeArea())
            .navigationBarTitleDisplayMode(.inline)
        }
        .presentationDetents([.medium])
        .presentationDragIndicator(.visible)
    }
}
