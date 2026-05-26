import SwiftUI

struct PacketServersView: View {
    @ObservedObject var tunnelManager: TunnelManager
    @State private var sheetMode: ConfigurationSheetMode?

    private var isShowingActiveConfiguration: Bool {
        tunnelManager.isRunning || tunnelManager.telemetry.snapshot.tunnelActive
    }

    var body: some View {
        NavigationStack {
            Group {
                if tunnelManager.savedConfigurations.isEmpty {
                    EmptyConfigurationState()
                } else {
                    List {
                        Section {
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
                        } footer: {
                            if tunnelManager.hasPendingSelectedConfiguration {
                                Text("Reconnect to apply selected server.")
                                    .foregroundStyle(.orange)
                            }
                        }
                    }
                    .listStyle(.insetGrouped)
                }
            }
            .navigationTitle("Servers")
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

// MARK: - Editor Sheet
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

                Section {
                    TextField(
                        "", text: $draftName,
                        prompt: Text(draftConfiguration.suggestedName).foregroundColor(.secondary)
                    )
                    .textInputAutocapitalization(.words)
                    .foregroundStyle(.primary)

                    Picker("Stack Mode", selection: $draftConfiguration.stackMode) {
                        ForEach(TunnelStackMode.allCases) { mode in
                            Text(mode.title).tag(mode)
                        }
                    }
                } header: {
                    Text("Profile Details")
                }

                if draftConfiguration.usesCustomCarrier {
                    Section {
                        TextField("trojan://password@edge:443?type=tcp&security=tls&fp=chrome", text: $draftConfiguration.trojanCarrierURI)
                            .keyboardType(.URL)
                            .textInputAutocapitalization(.never)
                            .autocorrectionDisabled()
                            .foregroundStyle(.primary)

                        HStack {
                            Text("DirectSock Local Port")
                            Spacer()
                            TextField("10808", text: $draftConfiguration.carrierProxyPort)
                                .keyboardType(.numberPad)
                                .multilineTextAlignment(.trailing)
                                .foregroundStyle(.secondary)
                        }
                    } header: {
                        Text("DirectSock Trojan")
                    }

                    Section {
                        Toggle("TLS Fragmentation", isOn: $draftConfiguration.fragmentEnabled)

                        if draftConfiguration.fragmentEnabled {
                            HStack {
                                Text("Fragment Size")
                                Spacer()
                                TextField("100", text: $draftConfiguration.fragmentSize)
                                    .keyboardType(.numberPad)
                                    .multilineTextAlignment(.trailing)
                                    .foregroundStyle(.secondary)
                            }
                        }
                    } header: {
                        Text("Advanced")
                    } footer: {
                        Text("Trojan TCP/TLS or WS/TLS link.")
                    }
                } else {
                    Section {
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
                    } header: {
                        Text("Server Info")
                    }

                    Section {
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

                        if draftConfiguration.transportMode == .obfs {
                            HStack {
                                Text("Obfs Key")
                                Spacer()
                                SecureField("Optional", text: $draftConfiguration.obfsKey)
                                    .multilineTextAlignment(.trailing)
                                    .foregroundStyle(.secondary)
                                    .textInputAutocapitalization(.never)
                                    .autocorrectionDisabled()
                            }

                            HStack {
                                Text("First-Hop Proxy")
                                Spacer()
                                TextField("Optional", text: $draftConfiguration.upstreamProxy)
                                    .multilineTextAlignment(.trailing)
                                    .foregroundStyle(.secondary)
                                    .textInputAutocapitalization(.never)
                                    .autocorrectionDisabled()
                            }
                        }
                    } header: {
                        Text("Network")
                    }

                    Section {
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

                        HStack {
                            Text("SNI Override")
                            Spacer()
                            TextField("Optional", text: $draftConfiguration.sniOverride)
                                .multilineTextAlignment(.trailing)
                                .foregroundStyle(.secondary)
                                .textInputAutocapitalization(.never)
                                .autocorrectionDisabled()
                        }

                        Toggle("TLS Fragmentation", isOn: $draftConfiguration.fragmentEnabled)

                        if draftConfiguration.fragmentEnabled {
                            HStack {
                                Text("Fragment Size")
                                Spacer()
                                TextField("40", text: $draftConfiguration.fragmentSize)
                                    .keyboardType(.numberPad)
                                    .multilineTextAlignment(.trailing)
                                    .foregroundStyle(.secondary)
                            }
                        }
                    } header: {
                        Text("Advanced")
                    } footer: {
                        Text("Leave optional fields blank unless required by your network administrator.")
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
        draftConfiguration.advancedValidationError == nil
    }

    private func saveConfiguration() {
        if let advancedValidationError = draftConfiguration.advancedValidationError {
            settingsError = advancedValidationError
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
        VStack(spacing: 10) {
            Image(systemName: "server.rack")
                .font(.system(size: 32, weight: .regular))
                .foregroundStyle(.secondary)

            Text("Empty")
                .font(.headline)
                .foregroundStyle(.primary)

            Text("Tap + to add a server.")
                .font(.subheadline)
                .foregroundStyle(.secondary)
        }
        .multilineTextAlignment(.center)
        .padding(24)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
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
