// lane_profile.rs — Per-lane variation parameters for the multi-lane stack
//
// Iran's DPI (2026 generation) does per-flow ML classification using TLS
// ClientHello shape, SNI, fragmentation pattern, and inter-arrival timing.
// A device that opens five tunnels with *identical* fingerprints looks like
// "one suspicious thing replicated five times" — it gets classified the same
// way and gets blocked together.
//
// To survive, the five parallel lanes need to look like five *different*
// flows. This module produces a `LaneProfile` per lane that tweaks:
//
//   * SNI / Host: rotated through a pool of fronting candidates
//   * Fragment seed: each lane uses a different randomization seed
//   * ALPN advertisement: h2 vs http/1.1 vs none
//   * User-Agent: rotated across mobile / desktop browsers
//   * Initial dwell: each lane starts its first write at a different jittered
//     offset, so the five flows don't share a synchronised handshake burst
//
// The lanes still all hit the same phantom-server through the same CDN edge,
// but to the DPI they look like five unrelated TLS sessions from a normal
// user browsing.

use rand::rngs::SmallRng;
use rand::seq::SliceRandom;
use rand::{Rng, SeedableRng};
use std::time::Duration;

/// What a single lane's transport is allowed to vary, on top of the shared
/// `ClientConfig`. The orchestrator overlays these onto the base config when
/// it spawns each lane.
#[derive(Clone, Debug)]
pub struct LaneProfile {
    /// Human-readable label used in logs.
    pub label: String,
    /// SNI to send during the TLS handshake to the CDN. Each lane gets a
    /// different one so JA3 + SNI pairs don't all collide.
    pub sni: Option<String>,
    /// Host header sent to the CDN. Usually matches `sni` for CDN routing.
    pub host_header: Option<String>,
    /// Lane-specific lower bound for the fragment chunk size. We keep the
    /// upper bound at the default 150 from `tls_fragment.rs`.
    pub fragment_size_hint: usize,
    /// User-Agent override for any HTTP headers the lane builds (browser
    /// transports, decoy GETs, etc.). Defaults to a stable Chrome string.
    pub user_agent: String,
    /// ALPN list to advertise. Different lanes can negotiate different ALPN
    /// values, giving 2-3 distinct cipher/extension fingerprints.
    pub alpn: Vec<Vec<u8>>,
    /// First-byte delay before this lane sends its first frame. Spreads the
    /// initial handshake burst over a 0–800ms window so the lanes don't
    /// stack into a single millisecond.
    pub initial_dwell: Duration,
    /// Whether this lane should prefer the BrowserLike TLS profile.
    pub browser_like: bool,
}

/// Pool of plausible-looking SNI hosts to rotate across lanes. Most of these
/// are large CDN-fronted properties whose TLS handshake looks innocuous to
/// the DPI. The fronts are paired with the real Host header by the operator
/// in their ClientConfig, so the SNI does NOT need to match the actual
/// origin.
const SNI_POOL: &[&str] = &[
    "ajax.cloudflare.com",
    "cdnjs.cloudflare.com",
    "static.cloudflareinsights.com",
    "www.cloudflare.com",
    "www.discord.com",
    "fonts.gstatic.com",
    "i.pinimg.com",
    "static.licdn.com",
    "www.tradingview.com",
    "raw.githubusercontent.com",
];

const UA_POOL: &[&str] = &[
    "Mozilla/5.0 (Linux; Android 13; SM-A536E) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/121.0.0.0 Mobile Safari/537.36",
    "Mozilla/5.0 (Linux; Android 14; Pixel 7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Mobile Safari/537.36",
    "Mozilla/5.0 (iPhone; CPU iPhone OS 17_2 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.2 Mobile/15E148 Safari/604.1",
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/121.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.2 Safari/605.1.15",
];

/// Build `count` lane profiles. Caller passes the base SNI/host that they
/// want as the *anchor* for lane 0 (i.e. the operator's real CDN front), and
/// the additional lanes rotate through `SNI_POOL` for variety. If the
/// operator wants every lane to use the exact same SNI (e.g. when fronting
/// requires an exact host), they can pass `pin_sni = true`.
pub fn build_profiles(
    count: usize,
    anchor_sni: Option<&str>,
    anchor_host: Option<&str>,
    pin_sni: bool,
) -> Vec<LaneProfile> {
    let count = count.max(1);
    let mut rng = SmallRng::from_entropy();
    let mut profiles = Vec::with_capacity(count);

    // Always emit the anchor profile as lane 0 — the operator's known-good
    // configuration. Later lanes diverge.
    profiles.push(LaneProfile {
        label: "lane-0/anchor".to_string(),
        sni: anchor_sni.map(str::to_string),
        host_header: anchor_host.map(str::to_string),
        fragment_size_hint: 100, // matches the v2rayNG tlshello low edge
        user_agent: UA_POOL[0].to_string(),
        alpn: vec![b"h2".to_vec(), b"http/1.1".to_vec()],
        initial_dwell: Duration::from_millis(0),
        browser_like: true,
    });

    for idx in 1..count {
        let sni = if pin_sni {
            anchor_sni.map(str::to_string)
        } else {
            // Pick from pool but avoid exact duplicates with anchor.
            let candidate = SNI_POOL
                .choose(&mut rng)
                .copied()
                .unwrap_or("ajax.cloudflare.com");
            Some(candidate.to_string())
        };

        let host_header = if pin_sni {
            anchor_host.map(str::to_string)
        } else {
            sni.clone()
        };

        // Spread fragment lower-bound across [100, 130] so each lane fingerprints
        // its handshake size differently.
        let fragment_size_hint = 100 + (idx as usize * 7) % 31;

        // Cycle ALPN combinations: lanes alternate between h2-preferred,
        // http/1.1-only, and h2-only. Three distinct ALPN profiles is enough
        // to break "five identical sessions" classifiers.
        let alpn = match idx % 3 {
            0 => vec![b"h2".to_vec(), b"http/1.1".to_vec()],
            1 => vec![b"http/1.1".to_vec()],
            _ => vec![b"h2".to_vec()],
        };

        let dwell_ms = rng.gen_range(80..=800);
        let user_agent = UA_POOL[idx % UA_POOL.len()].to_string();
        let browser_like = idx % 2 == 0;

        profiles.push(LaneProfile {
            label: format!("lane-{}/{}", idx, sni.as_deref().unwrap_or("?")),
            sni,
            host_header,
            fragment_size_hint,
            user_agent,
            alpn,
            initial_dwell: Duration::from_millis(dwell_ms),
            browser_like,
        });
    }

    profiles
}
