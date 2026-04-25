//! Invisible relay forwarder — the core data plane of the mesh.
//!
//! This module handles receiving onion-encrypted fragments from peers
//! and forwarding them to the next hop. It runs silently inside the
//! app's normal sync loop — the user never sees any indication.

use crate::heuristics;
use phantom_proto::onion;
use std::collections::VecDeque;
use std::time::Instant;

/// A fragment queued for relay forwarding.
#[derive(Debug, Clone)]
pub struct RelayFragment {
    /// The onion-encrypted payload (after peeling our layer).
    pub payload: Vec<u8>,
    /// The peer_id of the next hop.
    pub next_peer_id: String,
    /// When this fragment was received (for latency tracking).
    pub received_at: Instant,
    /// Whether this is cover traffic (affects priority).
    pub is_cover: bool,
}

/// The relay forwarder state, shared between the sync loop and the mesh.
pub struct RelayForwarder {
    /// Queued fragments waiting to be sent in the next sync cycle.
    outbound_queue: VecDeque<RelayFragment>,
    /// Our relay key for peeling onion layers addressed to us.
    relay_key: [u8; 32],
    /// Maximum fragments to forward per sync cycle (bandwidth control).
    max_per_cycle: usize,
}

impl RelayForwarder {
    pub fn new(relay_key: [u8; 32], max_per_cycle: usize) -> Self {
        Self {
            outbound_queue: VecDeque::new(),
            relay_key,
            max_per_cycle,
        }
    }

    /// Process an incoming onion-wrapped fragment.
    /// Peels our layer and either:
    /// - Queues it for forwarding to the next hop
    /// - Returns the inner payload if we are the final hop
    pub fn process_incoming(&mut self, data: &[u8]) -> Result<IncomingResult, &'static str> {
        let (remaining_hops, inner) = onion::onion_peel(data, &self.relay_key)?;

        if remaining_hops <= 1 {
            // We are the final hop — this payload is for us (or for the exit bridge)
            return Ok(IncomingResult::ForUs(inner));
        }

        // We are an intermediate hop — queue for forwarding.
        // The inner payload is still onion-encrypted for the remaining hops.
        // We don't know (and can't know) who the next hop is from the encrypted payload.
        // The routing table is managed externally — the caller must specify the next hop.
        Ok(IncomingResult::Forward(inner))
    }

    /// Queue a fragment for forwarding to a specific next hop.
    pub fn queue_forward(&mut self, payload: Vec<u8>, next_peer_id: String, is_cover: bool) {
        self.outbound_queue.push_back(RelayFragment {
            payload,
            next_peer_id,
            received_at: Instant::now(),
            is_cover,
        });
    }

    /// Drain up to `max_per_cycle` fragments from the queue.
    /// Called during the sync loop to mix relay fragments with own traffic.
    pub fn drain_outbound(&mut self) -> Vec<RelayFragment> {
        let count = self.outbound_queue.len().min(self.max_per_cycle);
        self.outbound_queue.drain(..count).collect()
    }

    /// Generate cover traffic fragments and queue them.
    pub fn inject_cover_traffic(&mut self, peer_ids: &[String]) {
        if peer_ids.is_empty() {
            return;
        }
        let real_count = self.outbound_queue.len();
        let (cover_count, _delay_ms) = heuristics::cover_traffic_plan(real_count);

        let mut rng = rand::thread_rng();
        use rand::Rng;
        for _ in 0..cover_count {
            let cover_payload = heuristics::generate_cover_payload();
            let target_idx = rng.gen_range(0..peer_ids.len());
            self.queue_forward(cover_payload, peer_ids[target_idx].clone(), true);
        }
    }
}

/// Result of processing an incoming onion fragment.
pub enum IncomingResult {
    /// Payload is for us (final hop) — process as application data.
    ForUs(Vec<u8>),
    /// Payload needs forwarding to the next hop.
    Forward(Vec<u8>),
}
