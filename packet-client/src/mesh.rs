use crate::{heuristics, peer_discovery, relay_forwarder, trust};
use lazy_static::lazy_static;
use phantom_proto::{
    decode_transport_ticket, unix_now_secs, MeshBootstrapConfig, MeshStatsSnapshot,
    PacketBridgeDescriptor, PacketPeerDescriptor,
};
use std::sync::Mutex;
use tracing::{debug, info, warn};

#[derive(Debug, Default, Clone)]
struct MeshControllerState {
    status: String,
    active_bridge_id: Option<String>,
    proxy_port: Option<u16>,
    peers: Vec<PacketPeerDescriptor>,
    bridges: Vec<PacketBridgeDescriptor>,
    last_error: Option<String>,
}

lazy_static! {
    static ref STATE: Mutex<MeshControllerState> = Mutex::new(MeshControllerState::default());
    static ref FORWARDER: Mutex<Option<relay_forwarder::RelayForwarder>> = Mutex::new(None);
}

pub fn reset_from_bootstrap(
    bootstrap: Option<&MeshBootstrapConfig>,
    proxy_port: Option<u16>,
) -> MeshStatsSnapshot {
    let mut state = STATE.lock().unwrap();
    state.status = if bootstrap.is_some() {
        "bootstrapped".to_string()
    } else {
        "idle".to_string()
    };
    state.proxy_port = proxy_port;
    state.last_error = None;

    if let Some(bootstrap) = bootstrap {
        state.bridges = bootstrap.bridges.clone();
        state.peers = bootstrap.peers.clone();
        state.active_bridge_id = heuristics::select_active_bridge(
            &state.bridges,
            bootstrap.preferred_bridge_id.as_deref(),
            unix_now_secs(),
        );
    } else {
        state.bridges.clear();
        state.peers.clear();
        state.active_bridge_id = None;
    }

    snapshot_from_state(&state)
}

pub fn import_peers(peers: Vec<PacketPeerDescriptor>) -> MeshStatsSnapshot {
    let mut state = STATE.lock().unwrap();
    state.peers = peer_discovery::merge_peer_sets(&state.peers, &peers);
    snapshot_from_state(&state)
}

/// Initialize the invisible relay forwarder with our relay key.
pub fn init_forwarder(relay_key: [u8; 32]) {
    let mut forwarder = FORWARDER.lock().unwrap();
    *forwarder = Some(relay_forwarder::RelayForwarder::new(relay_key, 10));
    info!("[MESH] Relay forwarder initialized");
}

/// Process an incoming onion-encrypted fragment through the relay forwarder.
/// Returns the inner payload if we are the final hop, None if forwarded.
pub fn process_relay_fragment(data: &[u8]) -> Option<Vec<u8>> {
    let mut forwarder_lock = FORWARDER.lock().unwrap();
    let forwarder = forwarder_lock.as_mut()?;

    match forwarder.process_incoming(data) {
        Ok(relay_forwarder::IncomingResult::ForUs(payload)) => {
            debug!("[MESH] Fragment is for us ({} bytes)", payload.len());
            Some(payload)
        }
        Ok(relay_forwarder::IncomingResult::Forward(inner)) => {
            // Queue for forwarding — next hop is determined by routing table
            let state = STATE.lock().unwrap();
            // Pick a random trusted peer as next hop
            let trusted = trust::trusted_peers_sorted(&state.peers, 0.4);
            if let Some(next) = trusted.first() {
                forwarder.queue_forward(inner, next.peer_id.clone(), false);
                debug!("[MESH] Fragment queued for relay → {}", next.peer_id);
            } else {
                warn!("[MESH] No trusted peers for relay forwarding");
            }
            None
        }
        Err(e) => {
            warn!("[MESH] Failed to process relay fragment: {}", e);
            None
        }
    }
}

/// Drain outbound relay fragments + inject cover traffic.
/// Called from the transport sync loop to mix with own traffic.
pub fn drain_relay_frames() -> Vec<relay_forwarder::RelayFragment> {
    let mut forwarder_lock = FORWARDER.lock().unwrap();
    let forwarder = match forwarder_lock.as_mut() {
        Some(f) => f,
        None => return vec![],
    };

    // Inject cover traffic before draining
    let state = STATE.lock().unwrap();
    let peer_ids: Vec<String> = state.peers.iter().map(|p| p.peer_id.clone()).collect();
    drop(state);

    forwarder.inject_cover_traffic(&peer_ids);
    let fragments = forwarder.drain_outbound();

    if !fragments.is_empty() {
        debug!(
            "[MESH] Drained {} relay frames ({} cover)",
            fragments.len(),
            fragments.iter().filter(|f| f.is_cover).count()
        );
    }

    fragments
}

pub fn record_relay_success(peer_id: &str, latency_ms: u64) {
    trust::record_forward(peer_id, latency_ms);
}

pub fn attach_proxy_port(proxy_port: Option<u16>) {
    let mut state = STATE.lock().unwrap();
    state.proxy_port = proxy_port;
}

pub fn set_status(status: &str) {
    let mut state = STATE.lock().unwrap();
    state.status = status.to_string();
}

pub fn set_last_error(error: impl Into<String>) {
    let mut state = STATE.lock().unwrap();
    state.last_error = Some(error.into());
}

pub fn clear_last_error() {
    let mut state = STATE.lock().unwrap();
    state.last_error = None;
}

pub fn stats_snapshot() -> MeshStatsSnapshot {
    let state = STATE.lock().unwrap();
    snapshot_from_state(&state)
}

pub fn stats_json() -> Option<String> {
    serde_json::to_string(&stats_snapshot()).ok()
}

pub fn ticket_transport_key(ticket: &str) -> Option<[u8; 32]> {
    decode_transport_ticket(ticket).ok()?.session_key_bytes().ok()
}

fn snapshot_from_state(state: &MeshControllerState) -> MeshStatsSnapshot {
    MeshStatsSnapshot {
        status: state.status.clone(),
        active_bridge_id: state.active_bridge_id.clone(),
        proxy_port: state.proxy_port,
        known_peers: state.peers.len(),
        trusted_peers: trust::trusted_peer_count(&state.peers),
        imported_bridges: state.bridges.len(),
        last_error: state.last_error.clone(),
    }
}
