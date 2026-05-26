import SwiftUI
import UIKit

struct PacketSettingsView: View {
    @ObservedObject var tunnelManager: TunnelManager
    @ObservedObject var complianceStore: PacketComplianceStore
    @State private var confirmation: SettingsConfirmation?

    private var buildVersionLabel: String {
        let info = Bundle.main.infoDictionary ?? [:]
        let version = info["CFBundleShortVersionString"] as? String ?? "Unknown"
        let build = info["CFBundleVersion"] as? String ?? "Unknown"
        return "\(version) (\(build))"
    }

    private var activeConfiguration: TunnelConfiguration {
        tunnelManager.displayConfiguration
    }

    var body: some View {
        NavigationStack {
            List {
                Section {
                    NavigationLink(destination: PacketServersView(tunnelManager: tunnelManager)) {
                        SettingsSummaryRow(
                            systemImage: "server.rack",
                            title: "Server Profiles",
                            value: "\(tunnelManager.savedConfigurations.count)"
                        )
                    }

                    SettingsSummaryRow(
                        systemImage: "point.3.connected.trianglepath.dotted",
                        title: "Default Protocol",
                        value: activeConfiguration.stackMode.title
                    )

                    SettingsSummaryRow(
                        systemImage: "arrow.triangle.2.circlepath",
                        title: "Transport",
                        value: activeConfiguration.usesCustomCarrier
                            ? activeConfiguration.ingressLabel
                            : activeConfiguration.transportMode.title
                    )

                    SettingsSummaryRow(
                        systemImage: "rectangle.compress.vertical",
                        title: "TLS Fragmentation",
                        value: activeConfiguration.fragmentEnabled
                            ? "\(activeConfiguration.fragmentSizeValue) bytes"
                            : "Off"
                    )
                } header: {
                    Text("Configuration")
                } footer: {
                    Text("Profile, protocol, transport, and fragmentation settings are configured before connecting.")
                }

                Section {
                    Button(role: .destructive) {
                        confirmation = .resetSelectedConfiguration
                    } label: {
                        SettingsActionRow(
                            systemImage: "arrow.counterclockwise",
                            title: "Reset Selected Configuration"
                        )
                    }
                    .disabled(tunnelManager.isRunning || !tunnelManager.hasConnectableConfiguration)

                    Button(role: .destructive) {
                        confirmation = .deleteAllConfigurations
                    } label: {
                        SettingsActionRow(
                            systemImage: "trash",
                            title: "Delete All Server Profiles"
                        )
                    }
                    .disabled(tunnelManager.isRunning || tunnelManager.savedConfigurations.isEmpty)
                } header: {
                    Text("Configuration Actions")
                } footer: {
                    Text(tunnelManager.isRunning ? "Disconnect before changing saved configurations." : "Reset clears the selected profile. Delete removes every saved server profile from this device.")
                }

                Section {
                    SettingsSummaryRow(
                        systemImage: "info.circle",
                        title: "Version",
                        value: buildVersionLabel
                    )
                } header: {
                    Text("Version")
                }

                Section {
                    Link(destination: PacketLegalLinks.privacy) {
                        SettingsLinkRow(systemImage: "hand.raised", title: "Privacy Policy")
                    }

                    Link(destination: PacketLegalLinks.terms) {
                        SettingsLinkRow(systemImage: "doc.text", title: "Terms of Use")
                    }

                    Link(destination: PacketLegalLinks.support) {
                        SettingsLinkRow(systemImage: "questionmark.circle", title: "Support")
                    }
                } header: {
                    Text("Legal")
                } footer: {
                    Text(PacketComplianceCopy.settingsFooterText)
                }
            }
            .listStyle(.insetGrouped)
            .navigationTitle("Settings")
            .navigationBarTitleDisplayMode(.inline)
            .confirmationDialog(
                confirmation?.title ?? "",
                isPresented: Binding(
                    get: { confirmation != nil },
                    set: { isPresented in
                        if !isPresented {
                            confirmation = nil
                        }
                    }
                ),
                titleVisibility: .visible
            ) {
                if let confirmation {
                    Button(confirmation.confirmTitle, role: .destructive) {
                        perform(confirmation)
                    }

                    Button("Cancel", role: .cancel) {
                        self.confirmation = nil
                    }
                }
            } message: {
                if let confirmation {
                    Text(confirmation.message)
                }
            }
        }
    }

    private func perform(_ confirmation: SettingsConfirmation) {
        switch confirmation {
        case .resetSelectedConfiguration:
            tunnelManager.clearSelectedConfiguration()
        case .deleteAllConfigurations:
            tunnelManager.deleteAllConfigurations()
        }

        self.confirmation = nil
    }
}

// MARK: - Helper Views
private enum SettingsConfirmation {
    case resetSelectedConfiguration
    case deleteAllConfigurations

    var title: String {
        switch self {
        case .resetSelectedConfiguration:
            return "Reset selected configuration?"
        case .deleteAllConfigurations:
            return "Delete all server profiles?"
        }
    }

    var message: String {
        switch self {
        case .resetSelectedConfiguration:
            return "This clears the selected configuration on this device. Saved server profiles remain available."
        case .deleteAllConfigurations:
            return "This removes every saved server profile and stored profile secret from this device."
        }
    }

    var confirmTitle: String {
        switch self {
        case .resetSelectedConfiguration:
            return "Reset"
        case .deleteAllConfigurations:
            return "Delete All"
        }
    }
}

private enum PacketLegalLinks {
    static let privacy = URL(string: "https://packet-tunnels.github.io/public/privacy.html")!
    static let terms = URL(string: "https://packet-tunnels.github.io/public/terms.html")!
    static let support = URL(string: "https://packet-tunnels.github.io/public/support.html")!
}

private struct SettingsSummaryRow: View {
    let systemImage: String
    let title: String
    let value: String

    var body: some View {
        HStack(alignment: .center, spacing: 12) {
            Image(systemName: systemImage)
                .font(.system(size: 15, weight: .semibold))
                .foregroundStyle(.secondary)
                .frame(width: 20)

            Text(title)
                .font(.system(size: 15, weight: .regular))
                .foregroundStyle(.primary)

            Spacer()

            Text(value)
                .font(.system(size: 13, weight: .semibold))
                .foregroundStyle(.secondary)
                .lineLimit(1)
        }
        .padding(.vertical, 4)
    }
}

private struct SettingsActionRow: View {
    let systemImage: String
    let title: String

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: systemImage)
                .font(.system(size: 15, weight: .semibold))
                .frame(width: 20)

            Text(title)
                .font(.system(size: 15, weight: .regular))

            Spacer()
        }
        .padding(.vertical, 4)
    }
}

private struct SettingsLinkRow: View {
    let title: String
    let systemImage: String

    init(systemImage: String, title: String) {
        self.title = title
        self.systemImage = systemImage
    }

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: systemImage)
                .font(.system(size: 15, weight: .semibold))
                .foregroundStyle(.secondary)
                .frame(width: 20)

            Text(title)
                .font(.system(size: 15, weight: .regular))
                .foregroundStyle(.primary)

            Spacer()
        }
        .padding(.vertical, 4)
    }
}

struct AboutView: View {
    let buildVersionLabel: String

    var body: some View {
        List {
            Section {
                VStack(spacing: 16) {
                    Image(systemName: "shield.righthalf.filled")
                        .font(.system(size: 64))
                        .foregroundStyle(.primary)

                    Text("Packet")
                        .font(.largeTitle.bold())
                        .tracking(1.0)

                    Text("Version \(buildVersionLabel)")
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                }
                .frame(maxWidth: .infinity)
                .padding(.vertical, 32)
                .listRowBackground(Color.clear)
                .listRowInsets(EdgeInsets())
            }
        }
        .listStyle(.insetGrouped)
        .navigationTitle("About Packet")
        .navigationBarTitleDisplayMode(.inline)
        .background(Color(uiColor: .systemGroupedBackground).ignoresSafeArea())
    }
}
