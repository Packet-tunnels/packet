import SwiftUI

struct ContentView: View {
    @StateObject private var tunnelManager = TunnelManager()
    @ObservedObject private var logManager = LogManager.shared
    @State private var showSettings = false
    @State private var showCopiedToast = false

    var body: some View {
        ZStack(alignment: .bottom) {
            Color(.systemBackground)
                .ignoresSafeArea()

            ScrollView {
                VStack(spacing: 0) {
                    headerBar
                        .padding(.horizontal, 20)
                        .padding(.top, 12)
                        .padding(.bottom, 20)

                    statusCard
                        .padding(.horizontal, 20)
                        .padding(.bottom, 16)

                    if tunnelManager.state == .failed {
                        errorBanner
                            .padding(.horizontal, 20)
                            .padding(.bottom, 16)
                            .transition(.move(edge: .top).combined(with: .opacity))
                    }

                    logsView
                        .padding(.horizontal, 20)
                        .padding(.bottom, 120)
                }
            }
            .scrollDismissesKeyboard(.interactively)
            .animation(.easeInOut(duration: 0.25), value: tunnelManager.state)

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

    // MARK: - Header

    private var headerBar: some View {
        HStack {
            VStack(alignment: .leading, spacing: 2) {
                Text("Packet")
                    .font(.system(size: 20, weight: .semibold))
                Text(tunnelManager.lastResult)
                    .font(.system(size: 11, design: .monospaced))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }

            Spacer()

            Button {
                showSettings = true
            } label: {
                Image(systemName: "gearshape.fill")
                    .font(.system(size: 18))
                    .foregroundStyle(.secondary)
                    .frame(width: 40, height: 40)
                    .background(Color(.secondarySystemBackground))
                    .clipShape(Circle())
            }
        }
    }

    // MARK: - Status Card

    private var statusCard: some View {
        HStack(spacing: 14) {
            statusDot
                .frame(width: 10, height: 10)

            VStack(alignment: .leading, spacing: 2) {
                Text(statusTitle)
                    .font(.system(size: 14, weight: .semibold))
                Text(statusSubtitle)
                    .font(.system(size: 11, design: .monospaced))
                    .foregroundStyle(.secondary)
            }

            Spacer()

            Text(tunnelManager.state.rawValue.uppercased())
                .font(.system(size: 9, weight: .bold, design: .monospaced))
                .tracking(1.2)
                .foregroundStyle(statusColor)
                .padding(.horizontal, 10)
                .padding(.vertical, 5)
                .background(statusColor.opacity(0.12))
                .clipShape(Capsule())
        }
        .padding(16)
        .background(Color(.secondarySystemBackground))
        .clipShape(RoundedRectangle(cornerRadius: 16, style: .continuous))
    }

    @ViewBuilder
    private var statusDot: some View {
        Circle()
            .fill(statusColor)
            .overlay(
                Circle()
                    .fill(statusColor.opacity(0.35))
                    .scaleEffect(tunnelManager.state == .launching ? 2.2 : 1.0)
                    .animation(
                        tunnelManager.state == .launching
                            ? .easeInOut(duration: 0.8).repeatForever(autoreverses: true)
                            : .default,
                        value: tunnelManager.state
                    )
            )
    }

    private var statusTitle: String {
        switch tunnelManager.state {
        case .idle: return "Disconnected"
        case .launching: return "Connecting…"
        case .running: return "Tunnel Active"
        case .failed: return "Connection Failed"
        }
    }

    private var statusSubtitle: String {
        let cfg = tunnelManager.configuration
        let server = cfg.normalizedServerURL.isEmpty ? "—" : cfg.remoteAddress
        let port = cfg.listenPort.isEmpty ? "—" : cfg.listenPort
        return "\(server) · Local SOCKS :\(port)"
    }

    private var statusColor: Color {
        switch tunnelManager.state {
        case .idle: return .gray
        case .launching: return .orange
        case .running: return .green
        case .failed: return .red
        }
    }

    // MARK: - Error Banner

    private var errorBanner: some View {
        HStack(spacing: 10) {
            Image(systemName: "exclamationmark.triangle.fill")
                .foregroundStyle(.red)
                .font(.system(size: 15))

            Text(tunnelManager.lastResult)
                .font(.system(size: 12, weight: .medium))
                .foregroundStyle(.primary)
                .lineLimit(3)

            Spacer(minLength: 0)

            Button {
                withAnimation { tunnelManager.dismissError() }
            } label: {
                Image(systemName: "xmark")
                    .font(.system(size: 10, weight: .bold))
                    .foregroundStyle(.secondary)
                    .frame(width: 24, height: 24)
                    .background(Color(.tertiarySystemBackground))
                    .clipShape(Circle())
            }
        }
        .padding(14)
        .background(Color.red.opacity(0.08))
        .overlay(
            RoundedRectangle(cornerRadius: 14, style: .continuous)
                .stroke(Color.red.opacity(0.2), lineWidth: 1)
        )
        .clipShape(RoundedRectangle(cornerRadius: 14, style: .continuous))
    }

    // MARK: - Bottom Bar

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
                                .opacity(0.8)
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
        .background(
            Color(.systemBackground)
                .ignoresSafeArea(.all, edges: .bottom)
        )
    }

    private var buttonGradient: some ShapeStyle {
        if tunnelManager.isRunning {
            return AnyShapeStyle(
                LinearGradient(
                    colors: [Color(red: 0.85, green: 0.2, blue: 0.2), Color(red: 0.7, green: 0.1, blue: 0.1)],
                    startPoint: .top, endPoint: .bottom
                )
            )
        }
        return AnyShapeStyle(
            LinearGradient(
                colors: [Color(red: 0.2, green: 0.5, blue: 1.0), Color(red: 0.15, green: 0.35, blue: 0.85)],
                startPoint: .top, endPoint: .bottom
            )
        )
    }

    // MARK: - Logs

    private var logsView: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                Text("LOGS")
                    .font(.system(size: 10, weight: .bold, design: .monospaced))
                    .tracking(1.5)
                    .foregroundStyle(.secondary)

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
                VStack(alignment: .leading, spacing: 4) {
                    ForEach(Array(logManager.logs.suffix(50).enumerated()), id: \.offset) { _, log in
                        Text(log)
                            .font(.system(size: 10, design: .monospaced))
                            .foregroundStyle(.primary.opacity(0.65))
                            .textSelection(.enabled)
                    }
                }
            }
        }
        .padding(16)
        .background(Color(.secondarySystemBackground))
        .clipShape(RoundedRectangle(cornerRadius: 16, style: .continuous))
    }

    // MARK: - Copy Logs

    private func copyLogs() {
        let text = logManager.logs.joined(separator: "\n")
        UIPasteboard.general.string = text
        showCopiedToast = true
        let impactFeedback = UIImpactFeedbackGenerator(style: .light)
        impactFeedback.impactOccurred()
        DispatchQueue.main.asyncAfter(deadline: .now() + 1.5) {
            showCopiedToast = false
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

    // MARK: - Settings Sheet

    private var settingsSheet: some View {
        NavigationStack {
            Form {
                Section("Server") {
                    configRow(title: "Server URL", text: $tunnelManager.configuration.serverURL)
                    configSecureRow(title: "Shared Secret", text: $tunnelManager.configuration.secret)
                    configRow(
                        title: "Local SOCKS Port",
                        text: $tunnelManager.configuration.listenPort,
                        keyboard: .numberPad,
                        placeholder: "1080"
                    )
                }

                Section("Transport Mode") {
                    Picker("Transport", selection: $tunnelManager.configuration.transportMode) {
                        ForEach(TunnelTransportMode.allCases) { mode in
                            Text(mode.title).tag(mode)
                        }
                    }
                }

                Section("CDN / Relay") {
                    configRow(title: "CDN Edge", text: $tunnelManager.configuration.cdnEdge, placeholder: "Optional (e.g. 185.x.x.x:80)")
                    configRow(title: "Host Override", text: $tunnelManager.configuration.hostOverride, placeholder: "Optional (e.g. your-domain.com)")
                }
            }
            .navigationTitle("Configuration")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") { showSettings = false }
                }
            }
        }
    }

    private func configRow(
        title: String,
        text: Binding<String>,
        keyboard: UIKeyboardType = .default,
        placeholder: String = ""
    ) -> some View {
        TextField(placeholder.isEmpty ? title : placeholder, text: text)
            .font(.system(size: 14, design: .monospaced))
            .keyboardType(keyboard)
            .textInputAutocapitalization(.never)
            .autocorrectionDisabled()
    }

    private func configSecureRow(title: String, text: Binding<String>) -> some View {
        SecureInputRow(title: title, text: text)
    }
}

struct SecureInputRow: View {
    let title: String
    @Binding var text: String
    @State private var isVisible = false

    var body: some View {
        HStack(spacing: 12) {
            if isVisible {
                TextField(title, text: $text)
                    .font(.system(size: 14, design: .monospaced))
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
            } else {
                SecureField(title, text: $text)
                    .font(.system(size: 14, design: .monospaced))
            }
            
            Button(action: { isVisible.toggle() }) {
                Image(systemName: isVisible ? "eye.slash" : "eye")
                    .foregroundColor(.secondary)
                    .frame(minWidth: 24, minHeight: 24)
            }
            .buttonStyle(.borderless)
            
            if !text.isEmpty {
                Button(action: { text = "" }) {
                    Image(systemName: "xmark.circle.fill")
                        .foregroundColor(.secondary)
                        .frame(minWidth: 24, minHeight: 24)
                }
                .buttonStyle(.borderless)
            }
            
            Button(action: {
                if let pasted = UIPasteboard.general.string {
                    text = pasted
                }
            }) {
                Image(systemName: "doc.on.clipboard")
                    .foregroundColor(.secondary)
                    .frame(minWidth: 24, minHeight: 24)
            }
            .buttonStyle(.borderless)
        }
    }
}
