import SwiftUI

struct ContentView: View {
    @StateObject private var tunnelManager = TunnelManager()
    @ObservedObject private var logManager = LogManager.shared

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                configurationView
                advancedConfigView
                logsView
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 20)
            .padding(.top, 20)
            .padding(.bottom, 120)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
        .background(Color(.systemGroupedBackground).ignoresSafeArea())
        .scrollDismissesKeyboard(.interactively)
        .safeAreaInset(edge: .bottom) {
            executeButton
        }
    }

    private var configurationView: some View {
        VStack(alignment: .leading, spacing: 18) {
            Text("Configuration")
                .font(.headline)

            VStack(spacing: 16) {
                configField(title: "Server URL", text: $tunnelManager.configuration.serverURL)
                configSecureField(title: "Shared Secret", text: $tunnelManager.configuration.secret)
            }

            HStack(spacing: 24) {
                configField(title: "Listen Port", text: $tunnelManager.configuration.listenPort, keyboard: .numberPad)

                VStack(alignment: .leading, spacing: 8) {
                    Text("Transport")
                        .font(.system(size: 9, weight: .medium))
                        .tracking(1.5)
                        .foregroundStyle(.secondary)
                        .textCase(.uppercase)
                    Picker("", selection: $tunnelManager.configuration.transportMode) {
                        ForEach(TunnelTransportMode.allCases) { mode in
                            Text(mode.title).tag(mode)
                        }
                    }
                    .pickerStyle(.menu)
                    .tint(.primary)
                    .font(.system(size: 14, design: .monospaced))

                    Rectangle()
                        .frame(height: 1)
                        .foregroundStyle(Color.gray.opacity(0.3))
                        .padding(.top, 4)
                }
            }
        }
        .padding(18)
        .background(Color(.secondarySystemBackground))
        .clipShape(RoundedRectangle(cornerRadius: 18, style: .continuous))
    }

    private var advancedConfigView: some View {
        VStack(alignment: .leading, spacing: 18) {
            Text("Advanced")
                .font(.headline)

            VStack(spacing: 16) {
                configField(title: "CDN Edge", text: $tunnelManager.configuration.cdnEdge)
                configField(title: "Host Override", text: $tunnelManager.configuration.hostOverride)
            }
        }
        .padding(18)
        .background(Color(.secondarySystemBackground))
        .clipShape(RoundedRectangle(cornerRadius: 18, style: .continuous))
    }

    private func configField(
        title: String,
        text: Binding<String>,
        keyboard: UIKeyboardType = .default
    ) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(title)
                .font(.system(size: 9, weight: .medium))
                .tracking(1.5)
                .foregroundStyle(.secondary)
                .textCase(.uppercase)
            TextField("", text: text)
                .font(.system(size: 14, design: .monospaced))
                .foregroundStyle(.primary)
                .keyboardType(keyboard)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .padding(.vertical, 4)
                .overlay(
                    Rectangle()
                        .frame(height: 1)
                        .foregroundStyle(Color.gray.opacity(0.3)),
                    alignment: .bottom
                )
        }
        .frame(maxWidth: .infinity)
    }

    private func configSecureField(title: String, text: Binding<String>) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(title)
                .font(.system(size: 9, weight: .medium))
                .tracking(1.5)
                .foregroundStyle(.secondary)
                .textCase(.uppercase)
            SecureField("", text: text)
                .font(.system(size: 14, design: .monospaced))
                .foregroundStyle(.primary)
                .padding(.vertical, 4)
                .overlay(
                    Rectangle()
                        .frame(height: 1)
                        .foregroundStyle(Color.gray.opacity(0.3)),
                    alignment: .bottom
                )
        }
        .frame(maxWidth: .infinity)
    }

    private var executeButton: some View {
        Button(action: tunnelManager.toggleTunnel) {
            Text(tunnelManager.primaryActionTitle)
                .font(.headline)
                .frame(maxWidth: .infinity)
                .padding(.vertical, 16)
                .foregroundStyle(.white)
                .background(tunnelManager.isRunning ? Color.red : Color.blue)
                .clipShape(RoundedRectangle(cornerRadius: 16, style: .continuous))
        }
        .disabled(tunnelManager.isBusy)
        .padding(.horizontal, 20)
        .padding(.vertical, 16)
        .background(.ultraThinMaterial)
    }

    private var logsView: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack {
                Text("Logs")
                    .font(.headline)
                Spacer()
                if !logManager.logs.isEmpty {
                    Button(action: tunnelManager.clearLogs) {
                        Text("Clear")
                            .font(.system(size: 9, weight: .medium))
                            .tracking(1.5)
                            .foregroundStyle(.secondary)
                            .textCase(.uppercase)
                    }
                }
            }

            if logManager.logs.isEmpty {
                Text("No logs yet.")
                    .font(.system(size: 12))
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, alignment: .center)
                    .padding(.vertical, 24)
            } else {
                VStack(alignment: .leading, spacing: 6) {
                    ForEach(Array(logManager.logs.suffix(30).enumerated()), id: \.offset) { _, log in
                        Text(log)
                            .font(.system(size: 10, design: .monospaced))
                            .foregroundStyle(.primary.opacity(0.7))
                            .textSelection(.enabled)
                    }
                }
            }
        }
        .padding(18)
        .background(Color(.secondarySystemBackground))
        .clipShape(RoundedRectangle(cornerRadius: 18, style: .continuous))
    }
}
