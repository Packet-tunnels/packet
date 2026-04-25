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
        VStack(spacing: 16) {
            HStack(spacing: 16) {
                VStack(alignment: .leading, spacing: 4) {
                    Text(statusHeadline.uppercased())
                        .font(.system(size: 11, weight: .bold, design: .monospaced))
                        .foregroundStyle(statusColor)
                        .tracking(1.0)

                    if tunnelManager.isRunning || tunnelManager.telemetry.snapshot.tunnelActive {
                        if let connectedSince = tunnelManager.telemetry.snapshot.connectedSince {
                            Text(Date(timeIntervalSince1970: TimeInterval(connectedSince)), style: .timer)
                                .font(.system(size: 20, weight: .semibold, design: .rounded).monospacedDigit())
                                .foregroundStyle(.primary)
                        } else {
                            Text("00:00")
                                .font(.system(size: 20, weight: .semibold, design: .rounded).monospacedDigit())
                                .foregroundStyle(.primary)
                        }
                    } else {
                        Text("Not Protected")
                            .font(.system(size: 20, weight: .semibold, design: .rounded))
                            .foregroundStyle(.primary)
                    }
                }

                Spacer()

                actionButton
            }

            Text(configurationSelectionText)
                .font(.system(size: 11, weight: .medium, design: .monospaced))
                .foregroundStyle(.secondary)
                .frame(maxWidth: .infinity, alignment: .leading)
                .fixedSize(horizontal: false, vertical: true)

            if let errorText = connectionErrorText {
                Text(errorText)
                    .font(.system(size: 13, weight: .medium))
                    .foregroundStyle(.red)
                    .multilineTextAlignment(.leading)
                    .fixedSize(horizontal: false, vertical: true)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(12)
                    .background(Color.red.opacity(0.1))
                    .clipShape(RoundedRectangle(cornerRadius: 12))
            }
        }
        .padding(20)
        .background(Color(uiColor: .secondarySystemGroupedBackground))
        .clipShape(RoundedRectangle(cornerRadius: 24, style: .continuous))
        .shadow(color: Color.black.opacity(0.04), radius: 12, x: 0, y: 6)
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

                Text(PacketComplianceCopy.reminderText)
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
        let configuration = tunnelManager.displayConfiguration

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
            return "Connected"
        case .launching:
            return "Connecting"
        case .failed:
            return "Failed"
        case .idle:
            return "Disconnected"
        }
    }

    private var connectionErrorText: String? {
        let snapshot = tunnelManager.telemetry.snapshot
        let configuration = tunnelManager.displayConfiguration
        if let lastError = snapshot.lastError?.nilIfEmpty {
            return lastError
        }
        if let cdnEdgeValidationError = configuration.cdnEdgeValidationError {
            return cdnEdgeValidationError
        }
        if tunnelManager.state == .failed {
            return tunnelManager.lastResult.nilIfEmpty ?? "Unknown connection error"
        }
        if !tunnelManager.isRunning,
            (configuration.normalizedServerURL.isEmpty || configuration.normalizedSecret.isEmpty)
        {
            return "Configuration incomplete. Add the server URL and shared secret in Settings."
        }
        return nil
    }

    private var configurationSelectionText: String {
        let selectedConfiguration = tunnelManager.selectedConfigurationDisplayName

        guard
            (tunnelManager.isRunning || tunnelManager.telemetry.snapshot.tunnelActive),
            let activeConfiguration = tunnelManager.activeConfigurationDisplayName
        else {
            return "Selected configuration: \(selectedConfiguration)"
        }

        if activeConfiguration == selectedConfiguration {
            return "Active configuration: \(activeConfiguration)"
        }

        return "Active now: \(activeConfiguration). Selected next: \(selectedConfiguration)"
    }

    private var transportMetricDetail: String {
        let snapshot = tunnelManager.telemetry.snapshot
        let configuration = tunnelManager.displayConfiguration
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
                    .font(.system(size: 16, weight: .semibold, design: .rounded))
                    .foregroundStyle(.primary)
                    .lineLimit(1)
                    .minimumScaleFactor(0.5)

                Text(detail)
                    .font(.system(size: 11, weight: .medium, design: .monospaced))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .minimumScaleFactor(0.5)
            }
        }
        .padding(16)
        .background(Color(uiColor: .secondarySystemGroupedBackground))
        .clipShape(RoundedRectangle(cornerRadius: 20, style: .continuous))
        .shadow(color: Color.black.opacity(0.02), radius: 10, x: 0, y: 4)
    }
}

struct PacketVPNDisclosureSheet: View {
    let isConnectFlow: Bool
    let acceptTitle: String
    let onAccept: () -> Void
    let onDismiss: () -> Void
    
    @Environment(\.colorScheme) var colorScheme
    @State private var isAcknowledged = false

    var body: some View {
        NavigationStack {
            VStack(spacing: 0) {
                ScrollView(.vertical, showsIndicators: false) {
                    VStack(alignment: .leading, spacing: 24) {
                        VStack(alignment: .leading, spacing: 12) {
                            Image(systemName: "checkmark.shield.fill")
                                .font(.system(size: 44))
                                .foregroundStyle(Color.green)
                                .padding(.bottom, 8)

                            Text("VPN Data-Use Disclosure")
                                .font(.system(size: 24, weight: .bold))
                                .foregroundStyle(.primary)

                            Text(PacketComplianceCopy.disclosureIntro)
                                .font(.system(size: 15))
                                .foregroundStyle(.secondary)
                                .lineSpacing(2)
                        }

                        VStack(spacing: 12) {
                            ForEach(PacketComplianceCopy.summaryItems) { item in
                                PacketDisclosureSummaryCard(item: item)
                            }
                        }

                        Text(PacketComplianceCopy.disclosureOutro)
                            .font(.system(size: 14))
                            .foregroundStyle(.tertiary)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                    .padding(.horizontal, 28)
                    .padding(.top, 32)
                    .padding(.bottom, 20)
                }

                VStack(spacing: 16) {
                    Button(action: {
                        let feedback = UIImpactFeedbackGenerator(style: .light)
                        feedback.impactOccurred()
                        isAcknowledged.toggle()
                    }) {
                        HStack(spacing: 12) {
                            Image(systemName: isAcknowledged ? "checkmark.circle.fill" : "circle")
                                .font(.system(size: 22))
                                .foregroundStyle(isAcknowledged ? .green : .secondary)
                            
                            Text("I have reviewed and understand how my data is handled.")
                                .font(.system(size: 14, weight: .medium))
                                .foregroundStyle(.secondary)
                                .multilineTextAlignment(.leading)
                        }
                    }
                    .buttonStyle(.plain)
                    .padding(.horizontal, 4)

                    Button(action: onAccept) {
                        HStack(spacing: 8) {
                            Image(systemName: "checkmark.circle.fill")
                                .font(.system(size: 18, weight: .bold))
                            
                            Text(acceptTitle)
                                .font(.system(size: 17, weight: .bold, design: .rounded))
                        }
                        .frame(maxWidth: .infinity)
                        .padding(.vertical, 16)
                        .background(
                            isAcknowledged 
                            ? (colorScheme == .dark ? Color.white : Color.black) 
                            : Color.secondary.opacity(0.2)
                        )
                        .foregroundStyle(
                            isAcknowledged 
                            ? (colorScheme == .dark ? Color.black : Color.white) 
                            : Color.secondary
                        )
                        .clipShape(RoundedRectangle(cornerRadius: 18, style: .continuous))
                    }
                    .disabled(!isAcknowledged)

                    Button(isConnectFlow ? "Not Now" : "Dismiss", action: onDismiss)
                        .font(.system(size: 15, weight: .semibold))
                        .foregroundStyle(.secondary)
                        .padding(.top, 4)
                }
                .padding(28)
                .background(Color(uiColor: .secondarySystemGroupedBackground))
                .shadow(color: Color.black.opacity(0.05), radius: 10, x: 0, y: -5)
            }
            .frame(maxWidth: .infinity, alignment: .topLeading)
            .background(Color(uiColor: .systemGroupedBackground).ignoresSafeArea())
            .navigationBarTitleDisplayMode(.inline)
        }
        .presentationDetents([.large])
        .presentationDragIndicator(.visible)
    }
}

private struct PacketDisclosureSummaryCard: View {
    let item: PacketComplianceSummaryItem

    var body: some View {
        HStack(alignment: .top, spacing: 14) {
            Image(systemName: item.systemImage)
                .font(.system(size: 16, weight: .semibold))
                .foregroundStyle(.primary)
                .frame(width: 24)

            VStack(alignment: .leading, spacing: 4) {
                Text(item.title)
                    .font(.system(size: 14, weight: .semibold))
                    .foregroundStyle(.primary)

                Text(item.detail)
                    .font(.system(size: 13))
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }

            Spacer(minLength: 0)
        }
        .padding(16)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Color(uiColor: .secondarySystemGroupedBackground))
        .clipShape(RoundedRectangle(cornerRadius: 16, style: .continuous))
    }
}
