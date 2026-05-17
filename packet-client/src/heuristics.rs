use phantom_proto::PacketBridgeDescriptor;
use rand::Rng;

/// Select the best active bridge from the available set.
pub fn select_active_bridge(
    bridges: &[PacketBridgeDescriptor],
    preferred_bridge_id: Option<&str>,
    now_secs: u64,
) -> Option<String> {
    let mut candidates = bridges
        .iter()
        .filter(|bridge| {
            bridge
                .expires_at
                .map(|expires_at| expires_at > now_secs)
                .unwrap_or(true)
        })
        .collect::<Vec<_>>();

    if let Some(preferred_bridge_id) = preferred_bridge_id {
        if let Some(preferred) = candidates
            .iter()
            .find(|bridge| bridge.id == preferred_bridge_id)
            .map(|bridge| bridge.id.clone())
        {
            return Some(preferred);
        }
    }

    candidates.sort_by_key(|bridge| bridge.priority.unwrap_or(u32::MAX));
    candidates.first().map(|bridge| bridge.id.clone())
}

/// Generate a cover traffic payload.
/// This is random data padded to a standard size, indistinguishable from a real fragment.
pub fn generate_cover_payload() -> Vec<u8> {
    let mut rng = rand::thread_rng();
    // Cover payloads use the same size distribution as real fragments
    let size = match rng.gen_range(0u8..100) {
        0..=60 => 512,    // Small — matches text fragments
        61..=85 => 4096,  // Medium
        86..=95 => 16384, // Large — matches voice note fragments
        _ => 65536,       // XL — matches image fragments
    };
    let mut payload = vec![0u8; size];
    rng.fill(&mut payload[..]);
    payload
}

/// Decide how many cover packets to inject in a given sync cycle.
/// More cover = better security but more bandwidth.
/// Returns (count, delays_between_ms).
pub fn cover_traffic_plan(real_fragment_count: usize) -> (usize, u64) {
    let mut rng = rand::thread_rng();
    // Always send at least 1 cover packet, even with no real data.
    // Target ratio: ~2 cover packets per real packet.
    let count = if real_fragment_count == 0 {
        rng.gen_range(1..=3)
    } else {
        let cover = real_fragment_count * 2 + rng.gen_range(0..=2);
        cover.min(10) // Cap to prevent excessive bandwidth
    };
    let delay = rng.gen_range(20..100);
    (count, delay)
}
