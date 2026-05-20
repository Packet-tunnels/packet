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
/// Ports we try for the QUIC (UDP) tunnel. 443 is what WhatsApp / Meet /
/// Telegram video use, which is *why* this transport passes Iran's TCP-
/// only handshake filter — UDP/443 has to stay open for those apps. We
/// also probe 8443 + 53 since those tend to be open for DoH/DoT.
const QUIC_PORTS: &[u16] = &[443, 8443, 53];

/// Extra seed hosts to include in the candidate pool, alongside whatever
/// host the operator configured. These are *additional* origins that have
/// the same `phantom-server` deployed under different IPs — when one IP
/// burns the others stay reachable, exactly how Psiphon's embedded server
/// list works. Empty by default; populated either at compile time from
/// `PHANTOM_SERVER_POOL` (build-time env, comma-separated `host[:port]`)
/// or — more importantly — at runtime via the `bootstrap_servers` field
/// in `ClientConfig`, which the Android / iOS layer plumbs in.
const EXTRA_SEED_HOSTS: &[&str] = &[];

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

    // 1) Operator-configured base, exactly as supplied. First because it's
    //    the known-good combination the operator believed in.
    out.push(Candidate {
        label: "base/as-configured".to_string(),
        config: base.clone(),
    });

    // 2) Build the full host pool: the primary host the operator targets,
    //    plus every host in EXTRA_SEED_HOSTS (and any bootstrap_servers
    //    plumbed in via the config — see the merge below). Each gets the
    //    full port × transport sweep, so when one IP burns the others
    //    keep working — this is exactly Psiphon's "embedded server list"
    //    behaviour, just merged with the per-host transport diversity.
    let mut hosts: Vec<String> = Vec::new();
    if let Some(primary) = base_host(base) {
        hosts.push(primary);
    }
    for extra in base.bootstrap_servers.iter().chain(EXTRA_SEED_HOSTS.iter().copied().map(str::to_string).collect::<Vec<_>>().iter()) {
        let trimmed = extra.trim();
        if !trimmed.is_empty() && !hosts.iter().any(|h| h == trimmed) {
            hosts.push(trimmed.to_string());
        }
    }

    for host in &hosts {
        let host = host.as_str();
        let host_tag = if hosts.len() > 1 {
            format!("{}@", host)
        } else {
            String::new()
        };

        // a) Chrome-TLS WebSocket, fragmented, swept across CF TLS ports.
        for &port in TLS_PORTS {
            let mut c = with_edge(base, host, port);
            c.mode = TransportMode::WebSocket;
            c.tls_profile = TlsProfile::BrowserLike;
            c.fragment_enabled = true;
            out.push(Candidate {
                label: format!("{}ws+chrome+frag :{}", host_tag, port),
                config: c,
            });
        }
        // b) Same, fragmentation OFF (some middleboxes flag the fragmenter
        //    itself; a clean Chrome hello sometimes passes where frag doesn't).
        for &port in &[443u16, 8443, 2053] {
            let mut c = with_edge(base, host, port);
            c.mode = TransportMode::WebSocket;
            c.tls_profile = TlsProfile::BrowserLike;
            c.fragment_enabled = false;
            out.push(Candidate {
                label: format!("{}ws+chrome+nofrag :{}", host_tag, port),
                config: c,
            });
        }
        // c) No-TLS Obfs (OSSH-style uniform-random bytes) across a wide
        //    port set — nothing for a TLS/SNI classifier to fire on.
        for &port in OBFS_PORTS {
            let mut c = with_edge(base, host, port);
            c.mode = TransportMode::Obfs;
            c.fragment_enabled = false;
            out.push(Candidate {
                label: format!("{}obfs :{}", host_tag, port),
                config: c,
            });
        }
        // d) QUIC (UDP) tunnel — the path that passes Iran's "TCP-handshake
        //    RST" filter because UDP/443 carries WhatsApp / Meet / Telegram
        //    video and can't be wholesale blackholed.
        for &port in QUIC_PORTS {
            let mut c = with_edge(base, host, port);
            c.mode = TransportMode::Quic;
            c.fragment_enabled = false;
            out.push(Candidate {
                label: format!("{}quic/udp :{}", host_tag, port),
                config: c,
            });
        }
        // e) Plaintext WebSocket on :80 (CDN HTTP path) — last resort, but
        //    occasionally the only thing a strict TLS filter leaves open.
        {
            let mut c = with_edge(base, host, 80);
            c.mode = TransportMode::WebSocket;
            c.fragment_enabled = false;
            out.push(Candidate {
                label: format!("{}ws+plain :80", host_tag),
                config: c,
            });
        }
    }

    out
}
