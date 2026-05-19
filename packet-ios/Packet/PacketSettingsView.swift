import SwiftUI
import UIKit

struct PacketSettingsView: View {
    @ObservedObject var tunnelManager: TunnelManager
    @ObservedObject var complianceStore: PacketComplianceStore

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

                Section {
                    NavigationLink(destination: AboutView(buildVersionLabel: buildVersionLabel)) {
                        SettingsSummaryRow(
                            systemImage: "info.circle",
                            title: "About Packet",
                            value: buildVersionLabel
                        )
                    }
                } header: {
                    Text("About")
                }
            }
            .listStyle(.insetGrouped)
            .navigationTitle("Settings")
            .navigationBarTitleDisplayMode(.inline)
        }
    }
}

// MARK: - Helper Views
private enum PacketLegalLinks {
    static let privacy = URL(string: "https://packet-tunnels.github.io/packet-public/privacy.html")!
    static let terms = URL(string: "https://packet-tunnels.github.io/packet-public/terms.html")!
    static let support = URL(string: "https://packet-tunnels.github.io/packet-public/support.html")!
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
