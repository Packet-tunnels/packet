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
                Label("Overview", systemImage: "wave.3.right.circle")
            }
            .tag(0)

            PacketSettingsView(tunnelManager: tunnelManager)
            .tabItem {
                Label("Settings", systemImage: "slider.horizontal.3")
            }
            .tag(1)
        }
        .tabViewStyle(.automatic)
    }
}

#Preview {
    PacketDashboardCompactView()
}
