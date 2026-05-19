// candidates.rs — Psiphon-style multi-candidate connection rotation
//
// The single biggest reason a working stack (v2ray+Psiphon+Conduit) takes up
// to ~10 minutes to connect while a naive client "fails fast" is that Psiphon
// does NOT retry one config — it grinds a large candidate pool (server × port
// × protocol × front) until one combination punches through Iran's DPI, then
// sticks with it. A single blocked config retried 1000× never helps; a
// different candidate every attempt eventually finds the hole.
//
// This module turns one base `TransportConfig` into an ordered candidate
// list: best-guess first (Chrome-TLS WS on 443, fragmented), then widening
// across ports, fragmentation, and the no-TLS Obfs path. The supervisor in
// `transport.rs` walks this list forever, never surfacing "failed" — only
// "still connecting (candidate i/N)", exactly like Psiphon's UI.

use crate::transport::{TlsProfile, TransportConfig, TransportMode};

/// One thing to try, with a human label for logs/telemetry.
pub struct Candidate {
    pub label: String,
    pub config: TransportConfig,
}

/// Ports to sweep. 443/8443/2053/2083/2087/2096 are Cloudflare-proxied TLS
/// ports (so a CF-fronted origin is reachable on any of them); 80 is the
/// plaintext-WS fallback; the high ports are for the no-TLS Obfs path where
/// Iran's well-known-port filters don't reach.
const TLS_PORTS: &[u16] = &[443, 8443, 2053, 2083, 2087, 2096];
const OBFS_PORTS: &[u16] = &[443, 80, 8443, 2053, 2087, 990, 8080];

/// Extract the bare host (no port) the base config is aimed at.
fn base_host(base: &TransportConfig) -> Option<String> {
    if let Some(edge) = base.cdn_edge.as_ref() {
        let h = edge.rsplit_once(':').map(|(h, _)| h).unwrap_or(edge);
        if !h.is_empty() {
            return Some(h.to_string());
        }
    }
    url::Url::parse(&base.server_url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
}

fn with_edge(base: &TransportConfig, host: &str, port: u16) -> TransportConfig {
    let mut c = base.clone();
    c.cdn_edge = Some(format!("{}:{}", host, port));
    c
}

/// Build the ordered candidate pool. Order = most-likely-to-work first so a
/// reachable path is usually found in the first minute, with the long tail
/// covering the stubborn cases over the next several minutes.
pub fn build_candidates(base: &TransportConfig) -> Vec<Candidate> {
    let mut out: Vec<Candidate> = Vec::new();
    let host = base_host(base);

    // 1) The base config exactly as configured — honour the operator's
    //    intended endpoint/SNI first.
    out.push(Candidate {
        label: "base/as-configured".to_string(),
        config: base.clone(),
    });

    if let Some(host) = host.as_deref() {
        // 2) Chrome-TLS WebSocket, fragmented, swept across CF TLS ports.
        for &port in TLS_PORTS {
            let mut c = with_edge(base, host, port);
            c.mode = TransportMode::WebSocket;
            c.tls_profile = TlsProfile::BrowserLike;
            c.fragment_enabled = true;
            out.push(Candidate {
                label: format!("ws+chrome+frag :{}", port),
                config: c,
            });
        }
        // 3) Same, fragmentation OFF (some middleboxes flag the fragmenter
        //    itself; a clean Chrome hello sometimes passes where frag doesn't).
        for &port in &[443u16, 8443, 2053] {
            let mut c = with_edge(base, host, port);
            c.mode = TransportMode::WebSocket;
            c.tls_profile = TlsProfile::BrowserLike;
            c.fragment_enabled = false;
            out.push(Candidate {
                label: format!("ws+chrome+nofrag :{}", port),
                config: c,
            });
        }
        // 4) No-TLS Obfs (OSSH-style uniform-random bytes) across a wide
        //    port set — nothing for a TLS/SNI classifier to fire on.
        for &port in OBFS_PORTS {
            let mut c = with_edge(base, host, port);
            c.mode = TransportMode::Obfs;
            c.fragment_enabled = false;
            out.push(Candidate {
                label: format!("obfs :{}", port),
                config: c,
            });
        }
        // 5) Plaintext WebSocket on :80 (CDN HTTP path) — last resort, but
        //    occasionally the only thing a strict TLS filter leaves open.
        {
            let mut c = with_edge(base, host, 80);
            c.mode = TransportMode::WebSocket;
            c.fragment_enabled = false;
            out.push(Candidate {
                label: "ws+plain :80".to_string(),
                config: c,
            });
        }
    }

    out
}
