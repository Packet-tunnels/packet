import SwiftUI
import UIKit

struct PacketSettingsView: View {
    @ObservedObject var tunnelManager: TunnelManager
    @ObservedObject var complianceStore: PacketComplianceStore

    @State private var showingDisclosureSheet = false

    private var buildVersionLabel: String {
        let info = Bundle.main.infoDictionary ?? [:]
        let version = info["CFBundleShortVersionString"] as? String ?? "Unknown"
        let build = info["CFBundleVersion"] as? String ?? "Unknown"
        return "\(version) (\(build))"
    }

    var body: some View {
        NavigationStack {
            List {
                Section {
                    NavigationLink(
                        destination: PrivacyDetailView(
                            title: "VPN Disclosure",
                            detail: PacketComplianceCopy.disclosureIntro + "\n\n"
                                + PacketComplianceCopy.disclosureOutro,
                            systemImage: "checkmark.shield.fill")
                    ) {
                        HStack(spacing: 12) {
                            Label("VPN Disclosure", systemImage: "checkmark.shield")
                                .foregroundStyle(.primary)

                            Spacer()

                            Text(
                                complianceStore.vpnDisclosureAcknowledged
                                    ? "Acknowledged" : "Review"
                            )
                            .font(.system(size: 13, weight: .semibold))
                            .foregroundStyle(
                                complianceStore.vpnDisclosureAcknowledged ? .green : .orange
                            )
                        }
                    }

                    ForEach(PacketComplianceCopy.summaryItems) { item in
                        NavigationLink(
                            destination: PrivacyDetailView(
                                title: item.title, detail: item.detail,
                                systemImage: item.systemImage)
                        ) {
                            PrivacySummaryRow(item: item)
                        }
                    }
                } header: {
                    Text("Privacy & Security")
                } footer: {
                    Text(PacketComplianceCopy.settingsFooterText)
                }

                Section {
                    NavigationLink(destination: AboutView(buildVersionLabel: buildVersionLabel)) {
                        HStack(spacing: 12) {
                            Image(systemName: "info.circle")
                                .foregroundStyle(.primary)
                            Text("About & Open Source")
                                .foregroundStyle(.primary)
                        }
                    }
                } header: {
                    Text("About Packet")
                }

                Section {
                    Button(action: {
                        if let url = URL(string: "https://github.com/Packet-tunnels/packet-public")
                        {
                            UIApplication.shared.open(url)
                        }
                    }) {
                        HStack(spacing: 12) {
                            Image(systemName: "link.circle.fill")
                                .font(.system(size: 20))
                                .foregroundStyle(.primary)

                            Text("Project Website & Source")
                                .foregroundStyle(.primary)
                        }
                    }
                } header: {
                    Text("Links")
                } footer: {
                    Text(
                        "Tap to visit our public Github repository to view docs and open source resources."
                    )
                }
            }
            .listStyle(.insetGrouped)
            .navigationTitle("Settings")
            .navigationBarTitleDisplayMode(.inline)
            .sheet(isPresented: $showingDisclosureSheet) {
                PacketVPNDisclosureSheet(
                    isConnectFlow: false,
                    acceptTitle: complianceStore.vpnDisclosureAcknowledged ? "Done" : "Acknowledge",
                    onAccept: {
                        complianceStore.setVPNDisclosureAcknowledged(true)
                        showingDisclosureSheet = false
                    },
                    onDismiss: {
                        showingDisclosureSheet = false
                    }
                )
            }
        }
    }
}

// MARK: - Helper Views
private struct PrivacySummaryRow: View {
    let item: PacketComplianceSummaryItem

    var body: some View {
        HStack(alignment: .center, spacing: 12) {
            Image(systemName: item.systemImage)
                .font(.system(size: 15, weight: .semibold))
                .foregroundStyle(.secondary)
                .frame(width: 20)

            Text(item.title)
                .font(.system(size: 15, weight: .regular))
                .foregroundStyle(.primary)
        }
        .padding(.vertical, 4)
    }
}

// MARK: - Detail Views
struct PrivacyDetailView: View {
    let title: String
    let detail: String
    let systemImage: String

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 24) {
                HStack(alignment: .center, spacing: 16) {
                    Image(systemName: systemImage)
                        .font(.system(size: 40))
                        .foregroundStyle(.primary)

                    Text(title)
                        .font(.title2.bold())
                }
                .padding(.bottom, 8)

                Text(detail)
                    .font(.body)
                    .foregroundStyle(.secondary)
                    .lineSpacing(4)
            }
            .padding(24)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .navigationTitle("Details")
        .navigationBarTitleDisplayMode(.inline)
        .background(Color(uiColor: .systemGroupedBackground).ignoresSafeArea())
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
