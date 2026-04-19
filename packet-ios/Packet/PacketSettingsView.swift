import SwiftUI
import UIKit

struct PacketSettingsView: View {
    @ObservedObject var tunnelManager: TunnelManager
    @ObservedObject var complianceStore: PacketComplianceStore

    @State private var sheetMode: ConfigurationSheetMode?
    @State private var showingDisclosureSheet = false

    private var isShowingActiveConfiguration: Bool {
        tunnelManager.isRunning || tunnelManager.telemetry.snapshot.tunnelActive
    }

    @Environment(\.editMode) private var editMode

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
                    if tunnelManager.savedConfigurations.isEmpty {
                        EmptyConfigurationState()
                            .listRowBackground(Color.clear)
                            .listRowInsets(EdgeInsets())
                    } else {
                        ForEach(tunnelManager.savedConfigurations) { savedConfiguration in
                            ConfigurationRow(
                                configuration: savedConfiguration,
                                isSelected: tunnelManager.selectedConfigurationID
                                    == savedConfiguration.id,
                                isActive: isShowingActiveConfiguration
                                    && tunnelManager.activeConfigurationID == savedConfiguration.id,
                                onEdit: {
                                    sheetMode = .edit(savedConfiguration)
                                }
                            )
                            .contentShape(Rectangle())
                            .onTapGesture {
                                tunnelManager.selectConfiguration(id: savedConfiguration.id)
                            }
                        }
                        .onDelete(perform: tunnelManager.deleteConfigurations)
                    }
                } header: {
                    Text("Saved Servers")
                } footer: {
                    if tunnelManager.hasPendingSelectedConfiguration {
                        Text("Reconnect to apply selected server.")
                            .foregroundStyle(.orange)
                    }
                }

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

                Section("About Packet") {
                    NavigationLink(destination: AboutView(buildVersionLabel: buildVersionLabel)) {
                        HStack(spacing: 12) {
                            Image(systemName: "info.circle")
                                .foregroundStyle(.primary)
                            Text("About & Open Source")
                                .foregroundStyle(.primary)
                        }
                    }
                }
            }
            .listStyle(.insetGrouped)  // Provides the modern, clean iOS list appearance
            .navigationTitle("Settings")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    if !tunnelManager.savedConfigurations.isEmpty {
                        EditButton()
                    }
                }

                ToolbarItem(placement: .topBarTrailing) {
                    Button(action: { sheetMode = .create }) {
                        Image(systemName: "plus")
                            .fontWeight(.medium)
                    }
                }
            }
            .sheet(item: $sheetMode) { mode in
                ConfigurationEditorSheet(
                    tunnelManager: tunnelManager,
                    mode: mode
                )
            }
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

// MARK: - Enums
private enum ConfigurationSheetMode: Identifiable {
    case create
    case edit(SavedTunnelConfiguration)

    var id: String {
        switch self {
        case .create: return "create"
        case .edit(let configuration): return configuration.id.uuidString
        }
    }

    var title: String {
        switch self {
        case .create: return "New Server"
        case .edit: return "Edit Server"
        }
    }

    var seedName: String {
        switch self {
        case .create: return ""
        case .edit(let configuration): return configuration.name
        }
    }

    var seedConfiguration: TunnelConfiguration {
        switch self {
        case .create: return TunnelConfiguration()
        case .edit(let configuration): return configuration.configuration
        }
    }
}

// MARK: - Editor Sheet (Refactored using standard Form)
private struct ConfigurationEditorSheet: View {
    @Environment(\.dismiss) private var dismiss

    @ObservedObject var tunnelManager: TunnelManager
    let mode: ConfigurationSheetMode

    @State private var draftName = ""
    @State private var draftConfiguration = TunnelConfiguration()
    @State private var settingsError: String?
    @State private var revealsSecret = false

    var body: some View {
        NavigationStack {
            Form {
                if let settingsError {
                    Section {
                        Text(settingsError)
                            .font(.system(size: 14, weight: .medium))
                            .foregroundStyle(.white)
                            .listRowBackground(Color.red)
                    }
                }

                Section(header: Text("Profile Details")) {
                    if #available(iOS 17.0, *) {
                        TextField(
                            "", text: $draftName,
                            prompt: Text(draftConfiguration.suggestedName).foregroundStyle(
                                .secondary)
                        )
                        .textInputAutocapitalization(.words)
                        .foregroundStyle(.primary)
                    } else {
                        // Fallback on earlier versions
                    }
                }

                Section(header: Text("Server Info")) {
                    TextField("https://example.com", text: $draftConfiguration.serverURL)
                        .keyboardType(.URL)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .foregroundStyle(.primary)

                    HStack {
                        if revealsSecret {
                            TextField("Shared Secret (Required)", text: $draftConfiguration.secret)
                        } else {
                            SecureField(
                                "Shared Secret (Required)", text: $draftConfiguration.secret)
                        }

                        Button(action: { revealsSecret.toggle() }) {
                            Image(systemName: revealsSecret ? "eye.slash" : "eye")
                                .foregroundColor(.secondary)
                        }
                        .buttonStyle(.plain)
                    }
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .foregroundStyle(.primary)
                }

                Section(header: Text("Network")) {
                    HStack {
                        Text("Listen Port")
                        Spacer()
                        TextField("Auto", text: $draftConfiguration.listenPort)
                            .keyboardType(.numberPad)
                            .multilineTextAlignment(.trailing)
                            .foregroundStyle(.secondary)
                    }

                    Picker("Transport", selection: $draftConfiguration.transportMode) {
                        ForEach(TunnelTransportMode.allCases) { mode in
                            Text(mode.title).tag(mode)
                        }
                    }
                }

                Section(
                    header: Text("Advanced"),
                    footer: Text(
                        "Leave optional fields blank unless required by your network administrator."
                    )
                ) {
                    HStack {
                        Text("CDN Edge")
                        Spacer()
                        TextField("Optional", text: $draftConfiguration.cdnEdge)
                            .multilineTextAlignment(.trailing)
                            .foregroundStyle(.secondary)
                            .textInputAutocapitalization(.never)
                            .autocorrectionDisabled()
                    }

                    HStack {
                        Text("Host Override")
                        Spacer()
                        TextField("Optional", text: $draftConfiguration.hostOverride)
                            .multilineTextAlignment(.trailing)
                            .foregroundStyle(.secondary)
                            .textInputAutocapitalization(.never)
                            .autocorrectionDisabled()
                    }
                }
            }
            .scrollDismissesKeyboard(.interactively)
            .navigationTitle(mode.title)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Button(action: { dismiss() }) {
                        Image(systemName: "xmark")
                            .font(.system(size: 14, weight: .bold))
                            .foregroundStyle(.primary)
                    }
                }

                ToolbarItem(placement: .topBarTrailing) {
                    Button("Save", action: saveConfiguration)
                        .font(.system(size: 16, weight: .bold))
                        .disabled(!canSave)
                }
            }
            .onAppear {
                draftName = mode.seedName
                draftConfiguration = mode.seedConfiguration
            }
        }
    }

    private var canSave: Bool {
        draftConfiguration.cdnEdgeValidationError == nil
    }

    private func saveConfiguration() {
        if let cdnEdgeValidationError = draftConfiguration.cdnEdgeValidationError {
            settingsError = cdnEdgeValidationError
            return
        }

        switch mode {
        case .create:
            tunnelManager.addConfiguration(named: draftName, configuration: draftConfiguration)
        case .edit(let configuration):
            tunnelManager.updateConfiguration(
                id: configuration.id,
                name: draftName,
                configuration: draftConfiguration
            )
        }

        settingsError = nil
        dismiss()
    }
}

// MARK: - Helper Views
private struct EmptyConfigurationState: View {
    var body: some View {
        VStack(alignment: .center, spacing: 12) {
            Image(systemName: "server.rack")
                .font(.system(size: 40))
                .foregroundColor(.secondary)

            Text("No configurations yet")
                .font(.system(size: 16, weight: .semibold))
                .foregroundStyle(.primary)

            Text(
                "Use the top-right add button to create a server profile, then select it before connecting."
            )
            .font(.system(size: 13))
            .foregroundStyle(.secondary)
            .multilineTextAlignment(.center)
        }
        .padding(.vertical, 32)
        .frame(maxWidth: .infinity)
    }
}

private struct ConfigurationRow: View {
    let configuration: SavedTunnelConfiguration
    let isSelected: Bool
    let isActive: Bool
    let onEdit: () -> Void

    var body: some View {
        HStack(spacing: 16) {
            if isActive {
                Image(systemName: "checkmark.circle.fill")
                    .font(.system(size: 22))
                    .foregroundStyle(.green)
            } else if isSelected {
                Image(systemName: "checkmark.circle.fill")
                    .font(.system(size: 22))
                    .foregroundStyle(.primary)
            } else {
                Circle()
                    .strokeBorder(Color.secondary.opacity(0.3), lineWidth: 1.5)
                    .frame(width: 22, height: 22)
            }

            VStack(alignment: .leading, spacing: 4) {
                Text(configuration.displayName)
                    .font(.system(size: 16, weight: .bold))
                    .foregroundStyle(.primary)

                if isActive {
                    Text("Connected")
                        .font(.system(size: 13, weight: .semibold))
                        .foregroundStyle(.green)
                } else if isSelected {
                    Text("Selected")
                        .font(.system(size: 13, weight: .semibold))
                        .foregroundStyle(.primary)
                } else {
                    Text(configuration.subtitle)
                        .font(.system(size: 13))
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
            }

            Spacer()

            Button(action: onEdit) {
                Image(systemName: "slider.horizontal.3")
                    .font(.system(size: 16, weight: .semibold))
                    .foregroundStyle(.primary)
            }
            .buttonStyle(.plain)
        }
        .padding(.vertical, 4)
    }
}

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
    @Environment(\.openURL) var openURL

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

            Section("Links") {
                Button(action: {
                    if let url = URL(string: "https://github.com/Packet-tunnels/packet-public") {
                        openURL(url)
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
            } footer: {
                Text(
                    "Tap to visit our public Github repository to view docs and open source resources."
                )
            }
        }
        .listStyle(.insetGrouped)
        .navigationTitle("About Packet")
        .navigationBarTitleDisplayMode(.inline)
        .background(Color(uiColor: .systemGroupedBackground).ignoresSafeArea())
    }
}
