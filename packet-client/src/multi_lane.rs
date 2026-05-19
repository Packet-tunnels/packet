// multi_lane.rs — Bonded multi-tunnel orchestrator
//
// The single-tunnel model (one WS+TLS connection from device to CDN) gets
// fingerprinted by Iran's DPI even with fragmentation. The fix observed in
// working stacked-tunnel profiles is that *multiple parallel
// tunnels* with different fingerprints survive where one clean tunnel dies.
//
// This module replicates that property in-process. We spawn N copies of the
// existing `transport::run_transport` loop — each with a different
// `LaneProfile` overlaid on the base `TransportConfig` — and route per-stream
// traffic across them. The DPI sees N independent TLS sessions, each with
// distinct SNI / ALPN / fragmentation, and cannot collapse them into a single
// blockable flow.
//
// Routing model
// ─────────────
// * Each lane runs its OWN `run_transport(...)` loop, which authenticates
//   and exchanges frames over its own private upstream channel pair.
// * `tunnel_state` is *shared* across all lanes. Downstream frames coming
//   back from any lane carry their `stream_id`, and the existing
//   `dispatch_downstream` helper looks up the right SOCKS5 handler by id.
// * A small `router` task receives the global SOCKS5 upstream and dispatches
//   each `UpstreamMsg` to the lane that owns its stream:
//     - `Connect{stream_id, ..}`: assigns a lane (round-robin) and remembers
//       the mapping.
//     - `Data{stream_id, ..}` / `Close{stream_id}`: forwards to the lane
//       previously assigned to that stream.
//
// Per-stream stickiness is critical: frames for one stream must arrive at
// the server in order through a single session, otherwise the
// per-session frame queue would observe gaps and stall.
//
// Resilience
// ──────────
// * If a lane's transport dies, its `run_transport` reconnects on its own
//   (existing behaviour). Streams that were assigned to the dead lane will
//   experience a brief stall; the existing 15-second SOCKS5 connect timeout
//   will surface the failure to the caller, which will retry naturally.
// * Decoy traffic (see `decoy.rs`) runs alongside, generating background
//   flows that further hide the tunnel.

use std::collections::HashMap;
use std::sync::Arc;

use rand::rngs::SmallRng;
use rand::{Rng, SeedableRng};
use tokio::sync::mpsc;
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

use crate::lane_profile::{build_profiles, LaneProfile};
use crate::transport::{self, TlsProfile, TransportConfig, TransportMode};
use crate::{TunnelState, UpstreamMsg};

/// Multi-lane orchestrator configuration.
#[derive(Clone, Debug)]
pub struct MultiLaneConfig {
    /// Number of parallel transport lanes to keep open.
    pub num_lanes: usize,
    /// If true, every lane uses the operator-supplied SNI. If false, lanes
    /// 1..N rotate through a pool of plausible fronting SNIs. Set this to
    /// true when the CDN requires an exact-match SNI for routing.
    pub pin_sni: bool,
}

impl Default for MultiLaneConfig {
    fn default() -> Self {
        Self {
            num_lanes: 5,
            pin_sni: false,
        }
    }
}

/// Spawn `num_lanes` transports in parallel and route SOCKS5 traffic across
/// them. Behaves like `transport::run_transport` from the caller's view: it
/// owns the global `upstream_rx` and writes downstream frames into the shared
/// `tunnel_state`.
pub async fn run_multi_lane(
    base_config: TransportConfig,
    lane_config: MultiLaneConfig,
    mut upstream_rx: mpsc::Receiver<UpstreamMsg>,
    tunnel_state: Arc<Mutex<TunnelState>>,
) {
    let num_lanes = lane_config.num_lanes.max(1);
    info!(
        "[multi-lane] starting {} parallel transport lanes (pin_sni={})",
        num_lanes, lane_config.pin_sni
    );

    let profiles = build_profiles(
        num_lanes,
        base_config.sni_override.as_deref(),
        base_config.host_header.as_deref(),
        lane_config.pin_sni,
    );

    // Spawn each lane with its overlaid config. Collect the per-lane senders
    // so the router can target individual lanes by index.
    let mut lane_senders: Vec<mpsc::Sender<UpstreamMsg>> = Vec::with_capacity(num_lanes);
    for (idx, profile) in profiles.iter().enumerate() {
        let (lane_tx, lane_rx) = mpsc::channel::<UpstreamMsg>(2048);
        let lane_cfg = build_lane_transport_config(&base_config, profile);
        let lane_state = tunnel_state.clone();
        let label = profile.label.clone();
        let dwell = profile.initial_dwell;
        info!(
            "[multi-lane] spawning {} (sni={:?}, alpn={:?}, fragment_hint={}, dwell={}ms, browser_like={})",
            label,
            lane_cfg.sni_override,
            profile.alpn.iter().map(|v| String::from_utf8_lossy(v).into_owned()).collect::<Vec<_>>(),
            lane_cfg.fragment_size,
            dwell.as_millis(),
            profile.browser_like,
        );
        tokio::spawn(async move {
            if !dwell.is_zero() {
                tokio::time::sleep(dwell).await;
            }
            debug!("[multi-lane] {} entering transport loop", label);
            transport::run_transport(lane_cfg, lane_rx, lane_state).await;
            warn!("[multi-lane] {} transport loop exited", label);
            let _ = idx; // silence unused
        });
        lane_senders.push(lane_tx);
    }

    // Per-stream → lane mapping. New streams are assigned via the
    // round-robin counter so traffic spreads evenly across lanes.
    let mut stream_lane: HashMap<u32, usize> = HashMap::new();
    let mut rr_counter: usize = 0;
    let mut rng = SmallRng::from_entropy();

    while let Some(msg) = upstream_rx.recv().await {
        // Pre-compute which lane handles this message before moving `msg`
        // into the send. CONNECT picks a new lane; DATA/CLOSE look it up.
        let (lane_idx, stream_id, is_connect) = match &msg {
            UpstreamMsg::Connect {
                stream_id, addr, ..
            } => {
                let jitter = rng.gen_range(0..num_lanes);
                let lane_idx = (rr_counter + jitter) % num_lanes;
                rr_counter = rr_counter.wrapping_add(1);
                stream_lane.insert(*stream_id, lane_idx);
                debug!(
                    "[multi-lane] stream {} CONNECT {} -> lane {}",
                    stream_id, addr, lane_idx
                );
                (Some(lane_idx), *stream_id, true)
            }
            UpstreamMsg::Data { stream_id, .. } => {
                let lane = stream_lane.get(stream_id).copied();
                if lane.is_none() {
                    debug!("[multi-lane] DATA for unknown stream {} dropped", stream_id);
                }
                (lane, *stream_id, false)
            }
            UpstreamMsg::Close { stream_id } => {
                let lane = stream_lane.remove(stream_id);
                (lane, *stream_id, false)
            }
        };

        let Some(lane_idx) = lane_idx else { continue };
        if lane_senders[lane_idx].send(msg).await.is_err() {
            warn!(
                "[multi-lane] lane {} dead while forwarding stream {} (connect={})",
                lane_idx, stream_id, is_connect
            );
            stream_lane.remove(&stream_id);
        }
    }

    info!("[multi-lane] global upstream channel closed, exiting orchestrator");
}

/// Build a `TransportConfig` for a single lane by overlaying the
/// `LaneProfile` onto the base config.
fn build_lane_transport_config(base: &TransportConfig, profile: &LaneProfile) -> TransportConfig {
    let mut cfg = base.clone();

    // SNI / Host overrides take precedence when set in the profile.
    if let Some(ref sni) = profile.sni {
        cfg.sni_override = Some(sni.clone());
    }
    if let Some(ref host) = profile.host_header {
        cfg.host_header = Some(host.clone());
    }

    // Fragment hint shifts the low edge of the chunk-size window. The
    // tls_fragment module only honours hints inside [100, 150).
    cfg.fragment_size = profile.fragment_size_hint;

    // TLS profile: half the lanes negotiate as browser-like, the other half
    // use the default rustls profile, so JA3 fingerprints split into two
    // populations instead of collapsing into one.
    cfg.tls_profile = if profile.browser_like {
        TlsProfile::BrowserLike
    } else {
        TlsProfile::Default
    };
    cfg.alpn_override = Some(profile.alpn.clone());
    cfg.user_agent_override = Some(profile.user_agent.clone());

    // We always force WebSocket transport for lanes — it's the path that
    // benefits most from fingerprint diversity. The auto/http fallbacks
    // would defeat the purpose by collapsing onto a single HTTP shape.
    if !matches!(cfg.mode, TransportMode::Http) {
        cfg.mode = TransportMode::WebSocket;
    }

    cfg
}
