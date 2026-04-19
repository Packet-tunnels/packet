import SwiftUI
import UIKit

struct PacketDashboardCompactView: View {
    @StateObject private var tunnelManager = TunnelManager()
    @StateObject private var complianceStore = PacketComplianceStore()
    @State private var selectedTab: Int = 0

    var body: some View {
        TabView(selection: $selectedTab) {
            PacketMainView(
                tunnelManager: tunnelManager,
                complianceStore: complianceStore
            )
            .tabItem {
                Label("Status", systemImage: tunnelManager.isRunning ? "checkmark.shield.fill" : "shield")
            }
            .tag(0)

            PacketServersView(
                tunnelManager: tunnelManager
            )
            .tabItem {
                Label("Servers", systemImage: "server.rack")
            }
            .tag(1)

            PacketSettingsView(
                tunnelManager: tunnelManager,
                complianceStore: complianceStore
            )
            .tabItem {
                Label("Settings", systemImage: "slider.horizontal.3")
            }
            .tag(2)
        }
        .tabViewStyle(.automatic)
    }
}

#Preview {
    PacketDashboardCompactView()
}
