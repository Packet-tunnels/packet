use phantom_proto::PacketPeerDescriptor;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

lazy_static::lazy_static! {
    static ref BEHAVIOR_LOG: Mutex<HashMap<String, PeerBehavior>> = Mutex::new(HashMap::new());
}

/// Tracks real-time relay behavior for trust decisions.
#[derive(Debug, Clone)]
pub struct PeerBehavior {
    pub forward_count: u64,
    pub drop_count: u64,
    pub total_latency_ms: u64,
    pub latency_samples: u32,
    pub latency_spikes: u32,
    pub cross_validations_passed: u32,
    pub cross_validations_failed: u32,
    pub first_seen: Instant,
}

impl Default for PeerBehavior {
    fn default() -> Self {
        let now = Instant::now();
        Self {
            forward_count: 0,
            drop_count: 0,
            total_latency_ms: 0,
            latency_samples: 0,
            latency_spikes: 0,
            cross_validations_passed: 0,
            cross_validations_failed: 0,
            first_seen: now,
        }
    }
}

impl PeerBehavior {
    fn avg_latency_ms(&self) -> f64 {
        if self.latency_samples == 0 {
            return 0.0;
        }
        self.total_latency_ms as f64 / self.latency_samples as f64
    }

    fn loss_rate(&self) -> f64 {
        let total = self.forward_count + self.drop_count;
        if total == 0 {
            return 0.0;
        }
        self.drop_count as f64 / total as f64
    }

    fn age_secs(&self) -> u64 {
        self.first_seen.elapsed().as_secs()
    }
}

/// Static trust score from descriptor metadata.
pub fn peer_trust_score(peer: &PacketPeerDescriptor) -> i32 {
    let base = peer.trust_score.unwrap_or(0);
    let relay_bonus = peer.relay_urls.len().min(4) as i32 * 5;
    let capability_bonus = peer.capabilities.len().min(4) as i32 * 3;
    base + relay_bonus + capability_bonus
}

pub fn trusted_peer_count(peers: &[PacketPeerDescriptor]) -> usize {
    peers
        .iter()
        .filter(|peer| peer_trust_score(peer) >= 10)
        .count()
}

/// Combined trust score: static descriptor score + behavioral score.
/// Returns 0.0 (definitely compromised) to 1.0 (fully trusted).
pub fn combined_trust(peer: &PacketPeerDescriptor) -> f32 {
    let static_score = (peer_trust_score(peer) as f32 / 50.0).clamp(0.0, 0.5);

    let behavioral_score = {
        let log = BEHAVIOR_LOG.lock().unwrap();
        match log.get(&peer.peer_id) {
            Some(behavior) => {
                let mut score = 0.0f32;

                // Reward: low loss rate
                let loss = behavior.loss_rate();
                score += if loss < 0.01 {
                    0.15
                } else if loss < 0.05 {
                    0.10
                } else if loss < 0.1 {
                    0.05
                } else {
                    0.0
                };

                // Reward: low latency
                let avg_ms = behavior.avg_latency_ms();
                score += if avg_ms < 50.0 {
                    0.1
                } else if avg_ms < 150.0 {
                    0.05
                } else {
                    0.0
                };

                // Reward: age (longer = more trusted)
                let age_hours = behavior.age_secs() / 3600;
                score += match age_hours {
                    0..=23 => 0.0,
                    24..=167 => 0.05,
                    168..=719 => 0.1,
                    _ => 0.15,
                };

                // Reward: cross-validation passes
                if behavior.cross_validations_passed > 0 && behavior.cross_validations_failed == 0 {
                    score += 0.1;
                }

                // Penalty: latency spikes (possible inspection)
                if behavior.latency_spikes > 5 {
                    score -= 0.15;
                }

                // Penalty: cross-validation failures
                if behavior.cross_validations_failed > 0 {
                    score -= 0.3;
                }

                score.clamp(0.0, 0.5)
            }
            None => 0.1, // Unknown peer — low behavioral trust
        }
    };

    (static_score + behavioral_score).clamp(0.0, 1.0)
}

/// Record a successful fragment forward through a relay.
pub fn record_forward(peer_id: &str, latency_ms: u64) {
    let mut log = BEHAVIOR_LOG.lock().unwrap();
    let behavior = log.entry(peer_id.to_string()).or_default();
    let avg = behavior.avg_latency_ms();
    behavior.forward_count += 1;
    behavior.total_latency_ms += latency_ms;
    behavior.latency_samples += 1;
    // Detect latency spike: >3x the running average
    if avg > 0.0 && latency_ms as f64 > avg * 3.0 {
        behavior.latency_spikes += 1;
    }
}

/// Get all peers with trust score above threshold, sorted by trust descending.
pub fn trusted_peers_sorted(peers: &[PacketPeerDescriptor], min_trust: f32) -> Vec<PacketPeerDescriptor> {
    let mut scored: Vec<_> = peers
        .iter()
        .map(|p| (combined_trust(p), p.clone()))
        .filter(|(trust, _)| *trust >= min_trust)
        .collect();
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    scored.into_iter().map(|(_, p)| p).collect()
}
