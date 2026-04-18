import SwiftUI

// MARK: - Main List View

struct PacketSettingsView: View {
    @ObservedObject var tunnelManager: TunnelManager
    @State private var showingConfigSheet = false
    
    // Using a mocked list to demonstrate the UI. In a real scenario,
    // this would iterate over an array in tunnelManager.
    @State private var mockConfigurations = [
        "Primary Edge Node",
        "Fallback Server EU"
    ]

    var body: some View {
        NavigationStack {
            ScrollView {
                LazyVStack(spacing: 16) {
                    ForEach(mockConfigurations, id: \.self) { configName in
                        ConfigurationCard(
                            title: configName,
                            subtitle: tunnelManager.configuration.serverURL.nilIfEmpty ?? "Not Configured",
                            isActive: configName == "Primary Edge Node"
                        )
                    }
                }
                .padding(.horizontal, 20)
                .padding(.vertical, 24)
            }
            .background(Color(uiColor: .systemGroupedBackground).ignoresSafeArea())
            .navigationTitle("Configurations")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    Button(action: { showingConfigSheet.toggle() }) {
                        Image(systemName: "plus")
                            .font(.system(size: 15, weight: .semibold))
                            .foregroundStyle(Color.primary)
                            .frame(width: 34, height: 34)
                            .background(Color(uiColor: .secondarySystemGroupedBackground))
                            .clipShape(Circle())
                            .shadow(color: Color.black.opacity(0.04), radius: 8, x: 0, y: 4)
                    }
                }
            }
            .sheet(isPresented: $showingConfigSheet) {
                ConfigurationEditorSheet(tunnelManager: tunnelManager)
            }
        }
    }
}

// MARK: - Configuration Editor Sheet

struct ConfigurationEditorSheet: View {
    @Environment(\.dismiss) var dismiss
    @ObservedObject var tunnelManager: TunnelManager
    @State private var draftConfiguration = TunnelConfiguration()
    @State private var settingsError: String?

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(spacing: 24) {
                    if let settingsError {
                        CompactSettingsErrorBanner(message: settingsError)
                    }

                    settingsForm
                }
                .padding(.horizontal, 20)
                .padding(.vertical, 24)
            }
            .background(Color(uiColor: .systemGroupedBackground).ignoresSafeArea())
            .scrollDismissesKeyboard(.interactively)
            .navigationTitle("New Server")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                        .foregroundStyle(Color.primary)
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Save") { saveConfiguration() }
                        .font(.system(size: 17, weight: .semibold))
                        .foregroundStyle(canSave ? Color.primary : Color.secondary)
                        .disabled(!canSave)
                }
            }
            .onAppear {
                draftConfiguration = tunnelManager.configuration
            }
        }
    }

    private var settingsForm: some View {
        VStack(spacing: 16) {
            CompactSettingsTextRow(
                label: "SERVER URL",
                text: $draftConfiguration.serverURL,
                placeholder: "https://example.com",
                keyboard: .URL
            )

            CompactSettingsSecretRow(
                label: "SHARED SECRET",
                placeholder: "Enter shared secret",
                text: $draftConfiguration.secret
            )

            HStack(alignment: .top, spacing: 16) {
                CompactSettingsHalfTextField(
                    label: "LISTEN PORT",
                    text: $draftConfiguration.listenPort,
                    placeholder: "Auto",
                    keyboard: .numberPad
                )

                CompactSettingsTransportRow(
                    label: "TRANSPORT",
                    selection: $draftConfiguration.transportMode
                )
            }

            HStack(alignment: .top, spacing: 16) {
                CompactSettingsHalfTextField(
                    label: "CDN EDGE",
                    text: $draftConfiguration.cdnEdge,
                    placeholder: "185.143.234.235:80"
                )

                CompactSettingsHalfTextField(
                    label: "HOST OVERRIDE",
                    text: $draftConfiguration.hostOverride,
                    placeholder: "your-domain.com"
                )
            }

            if let cdnEdgeValidationError = draftConfiguration.cdnEdgeValidationError {
                CompactSettingsErrorBanner(message: cdnEdgeValidationError)
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
        tunnelManager.configuration = draftConfiguration
        settingsError = nil
        dismiss()
    }
}

// MARK: - Reusable UI Components

struct ConfigurationCard: View {
    let title: String
    let subtitle: String
    let isActive: Bool

    var body: some View {
        HStack(spacing: 16) {
            VStack(alignment: .leading, spacing: 6) {
                Text(title)
                    .font(.system(size: 16, weight: .semibold, design: .default))
                    .foregroundStyle(Color.primary)
                
                Text(subtitle)
                    .font(.system(size: 13, weight: .regular))
                    .foregroundStyle(Color.secondary)
            }
            
            Spacer()
            
            if isActive {
                HStack(spacing: 6) {
                    Circle()
                        .fill(Color.green)
                        .frame(width: 6, height: 6)
                    
                    Text("Active")
                        .font(.system(size: 12, weight: .medium))
                        .foregroundStyle(Color.secondary)
                }
                .padding(.horizontal, 10)
                .padding(.vertical, 6)
                .background(Color.green.opacity(0.1))
                .clipShape(Capsule())
            } else {
                Image(systemName: "chevron.right")
                    .font(.system(size: 12, weight: .semibold))
                    .foregroundStyle(Color(uiColor: .tertiaryLabel))
            }
        }
        .padding(20)
        .background(Color(uiColor: .secondarySystemGroupedBackground))
        .clipShape(RoundedRectangle(cornerRadius: 18, style: .continuous))
        .shadow(color: Color.black.opacity(0.02), radius: 10, x: 0, y: 4)
    }
}

struct CompactSettingsErrorBanner: View {
    let message: String

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            Image(systemName: "exclamationmark.triangle.fill")
                .font(.system(size: 14))
                .foregroundStyle(Color.red.opacity(0.8))

            Text(message)
                .font(.system(size: 13, weight: .medium))
                .foregroundStyle(Color.primary)
                .fixedSize(horizontal: false, vertical: true)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(16)
        .background(Color.red.opacity(0.08))
        .clipShape(RoundedRectangle(cornerRadius: 16, style: .continuous))
    }
}

struct CompactSettingsTextRow: View {
    let label: String
    @Binding var text: String
    let placeholder: String
    var keyboard: UIKeyboardType = .default

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(label)
                .font(.system(size: 11, weight: .semibold))
                .foregroundStyle(Color.secondary)
                .tracking(0.5)

            TextField(placeholder, text: $text)
                .keyboardType(keyboard)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .font(.system(size: 16, weight: .regular))
                .foregroundStyle(Color.primary)
                .padding(.horizontal, 16)
                .padding(.vertical, 16)
                .background(Color(uiColor: .secondarySystemGroupedBackground))
                .clipShape(RoundedRectangle(cornerRadius: 14, style: .continuous))
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

struct CompactSettingsSecretRow: View {
    let label: String
    let placeholder: String
    @Binding var text: String
    @State private var revealsText = false

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(label)
                .font(.system(size: 11, weight: .semibold))
                .foregroundStyle(Color.secondary)
                .tracking(0.5)

            HStack(spacing: 12) {
                Group {
                    if revealsText {
                        TextField(placeholder, text: $text)
                    } else {
                        SecureField(placeholder, text: $text)
                    }
                }
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .font(.system(size: 16, weight: .regular))
                .foregroundStyle(Color.primary)

                Button(revealsText ? "Hide" : "Show") {
                    revealsText.toggle()
                }
                .font(.system(size: 13, weight: .medium))
                .foregroundStyle(Color.primary)
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 16)
            .background(Color(uiColor: .secondarySystemGroupedBackground))
            .clipShape(RoundedRectangle(cornerRadius: 14, style: .continuous))
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

struct CompactSettingsHalfTextField: View {
    let label: String
    @Binding var text: String
    let placeholder: String
    var keyboard: UIKeyboardType = .default

    var body: some View {
        CompactSettingsTextRow(
            label: label,
            text: $text,
            placeholder: placeholder,
            keyboard: keyboard
        )
    }
}

struct CompactSettingsTransportRow: View {
    let label: String
    @Binding var selection: TunnelTransportMode

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(label)
                .font(.system(size: 11, weight: .semibold))
                .foregroundStyle(Color.secondary)
                .tracking(0.5)

            Menu {
                ForEach(TunnelTransportMode.allCases) { mode in
                    Button(mode.title) {
                        selection = mode
                    }
                }
            } label: {
                HStack {
                    Text(selection.title)
                        .font(.system(size: 16, weight: .regular))
                        .foregroundStyle(Color.primary)

                    Spacer()

                    Image(systemName: "chevron.up.chevron.down")
                        .font(.system(size: 12, weight: .semibold))
                        .foregroundStyle(Color.secondary)
                }
                .padding(.horizontal, 16)
                .padding(.vertical, 16)
                .background(Color(uiColor: .secondarySystemGroupedBackground))
                .clipShape(RoundedRectangle(cornerRadius: 14, style: .continuous))
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

extension String {
    var nilIfEmpty: String? {
        let trimmedValue = trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmedValue.isEmpty ? nil : trimmedValue
    }
}