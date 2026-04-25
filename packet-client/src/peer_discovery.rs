use phantom_proto::PacketPeerDescriptor;
use std::collections::HashMap;

/// Merge two peer sets, deduplicating by peer_id.
/// Keeps the entry with the highest trust score and merges relay URLs/capabilities.
pub fn merge_peer_sets(
    current: &[PacketPeerDescriptor],
    incoming: &[PacketPeerDescriptor],
) -> Vec<PacketPeerDescriptor> {
    let mut merged = HashMap::new();

    for peer in current.iter().chain(incoming.iter()) {
        merged
            .entry(peer.peer_id.clone())
            .and_modify(|existing: &mut PacketPeerDescriptor| {
                if peer.trust_score.unwrap_or_default() > existing.trust_score.unwrap_or_default() {
                    existing.trust_score = peer.trust_score;
                }
                for relay_url in &peer.relay_urls {
                    if !existing.relay_urls.contains(relay_url) {
                        existing.relay_urls.push(relay_url.clone());
                    }
                }
                for capability in &peer.capabilities {
                    if !existing.capabilities.contains(capability) {
                        existing.capabilities.push(capability.clone());
                    }
                }
                existing.last_seen_at = peer.last_seen_at.or(existing.last_seen_at);
            })
            .or_insert_with(|| peer.clone());
    }

    merged.into_values().collect()
}
