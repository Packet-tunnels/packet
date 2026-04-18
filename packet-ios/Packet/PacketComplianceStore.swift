import Foundation

@MainActor
final class PacketComplianceStore: ObservableObject {
    @Published private(set) var vpnDisclosureAcknowledged: Bool

    init() {
        vpnDisclosureAcknowledged = SharedTunnelPreferenceStore.vpnDisclosureAcknowledged
    }

    func setVPNDisclosureAcknowledged(_ acknowledged: Bool) {
        guard acknowledged != vpnDisclosureAcknowledged else { return }
        SharedTunnelPreferenceStore.setVPNDisclosureAcknowledged(acknowledged)
        vpnDisclosureAcknowledged = acknowledged
    }
}
