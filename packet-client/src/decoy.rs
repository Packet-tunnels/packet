// decoy.rs — Background cover traffic generator
//
// Iran's DPI as of 2026 fingerprints devices that show a single long-lived
// outbound TLS flow with no other browsing alongside it. Working stacks
// Stacked-tunnel profiles implicitly defeat this by running several
// apps that each generate parallel flows to legitimate destinations.
//
// This module replicates that property natively: from the same process, we
// periodically open real TLS handshakes to popular Iranian domains, send a
// short HTTP/1.1 GET, read a few bytes, and tear the connection down. To the
// DPI the device looks like a normal Iranian user browsing Aparat / Digikala
// / Snapp while the tunnel runs in the background — which is exactly what a
// real user *is* doing.
//
// Important: decoys are NOT routed through the tunnel. They go direct to the
// Iranian domain so the source IP / timing / TLS exchange is genuine. The
// goal is *real flow noise on the same device*, not just spoofed traffic.

use std::sync::Arc;
use std::time::Duration;

use rand::rngs::SmallRng;
use rand::seq::SliceRandom;
use rand::{Rng, SeedableRng};
use rustls::ClientConfig as RustlsClientConfig;
use rustls::RootCertStore;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::watch;
use tokio::time::sleep;
use tokio_rustls::rustls::pki_types::ServerName;
use tokio_rustls::TlsConnector;
use tracing::{debug, trace};

/// Popular Iranian domains used as decoy destinations. All are HTTPS-capable
/// and reachable from inside Iran without circumvention. Mix of news,
/// e-commerce, ride-hailing, video, and blogs so the flow pattern looks like
/// natural browsing.
const DECOY_DOMAINS: &[&str] = &[
    "www.aparat.com",
    "www.digikala.com",
    "www.snapp.ir",
    "blu.ir",
    "www.divar.ir",
    "www.varzesh3.com",
    "tapsi.cab",
    "www.zoomit.ir",
    "www.bartarinha.ir",
    "www.shaparak.ir",
    "www.namnak.com",
    "www.balad.ir",
    "www.mihanblog.com",
    "www.bama.ir",
    "www.filimo.com",
    "www.tabnak.ir",
    "www.tasnimnews.com",
    "www.iribnews.ir",
    "www.farsnews.ir",
    "www.sahebkhabar.ir",
];

/// User-Agent strings rotated across decoy visits to vary fingerprints.
const USER_AGENTS: &[&str] = &[
    "Mozilla/5.0 (Linux; Android 13; SM-A536E) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/121.0.0.0 Mobile Safari/537.36",
    "Mozilla/5.0 (Linux; Android 14; Pixel 7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Mobile Safari/537.36",
    "Mozilla/5.0 (iPhone; CPU iPhone OS 17_2 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.2 Mobile/15E148 Safari/604.1",
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/121.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.2 Safari/605.1.15",
];

#[derive(Clone, Debug)]
pub struct DecoyConfig {
    /// Minimum seconds between decoy visits.
    pub min_interval_secs: u64,
    /// Maximum seconds between decoy visits.
    pub max_interval_secs: u64,
    /// Whether to run at all.
    pub enabled: bool,
    /// Number of concurrent decoy workers. Higher = more cover, more battery.
    pub workers: usize,
}

impl Default for DecoyConfig {
    fn default() -> Self {
        Self {
            min_interval_secs: 20,
            max_interval_secs: 75,
            enabled: true,
            workers: 2,
        }
    }
}

/// Spawn the decoy traffic generator. Returns immediately; lives until the
/// shutdown signal flips.
pub fn spawn_decoy_loop(config: DecoyConfig, shutdown: watch::Receiver<bool>) {
    if !config.enabled {
        debug!("[decoy] disabled");
        return;
    }
    let workers = config.workers.max(1);
    let tls_config = match build_decoy_tls_config() {
        Ok(cfg) => Arc::new(cfg),
        Err(e) => {
            debug!("[decoy] could not build TLS config: {} — decoy disabled", e);
            return;
        }
    };
    debug!(
        "[decoy] starting {} worker(s), interval {}-{}s",
        workers, config.min_interval_secs, config.max_interval_secs
    );
    for worker_idx in 0..workers {
        let cfg = config.clone();
        let tls = tls_config.clone();
        let shutdown = shutdown.clone();
        tokio::spawn(async move {
            run_decoy_worker(worker_idx, cfg, tls, shutdown).await;
        });
    }
}

async fn run_decoy_worker(
    worker_idx: usize,
    config: DecoyConfig,
    tls_config: Arc<RustlsClientConfig>,
    mut shutdown: watch::Receiver<bool>,
) {
    let mut rng = SmallRng::from_entropy();

    // Stagger the workers so they don't all fire on the same second.
    let initial_delay = Duration::from_secs(rng.gen_range(3..30) + (worker_idx as u64) * 7);
    tokio::select! {
        _ = sleep(initial_delay) => {}
        _ = shutdown.changed() => return,
    }

    loop {
        // Pick a random decoy and visit it. Errors are silently ignored — we
        // don't care if Aparat is briefly unreachable, the *attempt* and the
        // resulting TLS handshake are what matter for the cover.
        let domain = *DECOY_DOMAINS.choose(&mut rng).unwrap_or(&"www.aparat.com");
        let user_agent = *USER_AGENTS.choose(&mut rng).unwrap_or(&USER_AGENTS[0]);
        let _ = decoy_visit(domain, user_agent, tls_config.clone()).await;

        let lo = config.min_interval_secs.max(1);
        let hi = config.max_interval_secs.max(lo + 1);
        let wait = Duration::from_secs(rng.gen_range(lo..=hi));
        tokio::select! {
            _ = sleep(wait) => {}
            _ = shutdown.changed() => return,
        }
    }
}

fn build_decoy_tls_config() -> Result<RustlsClientConfig, String> {
    let mut roots = RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let cfg = RustlsClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(cfg)
}

async fn decoy_visit(
    domain: &str,
    user_agent: &str,
    tls_config: Arc<RustlsClientConfig>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let addr = format!("{}:443", domain);
    let tcp = tokio::time::timeout(Duration::from_secs(8), TcpStream::connect(&addr)).await??;
    let _ = tcp.set_nodelay(true);

    let connector = TlsConnector::from(tls_config);
    let server_name = ServerName::try_from(domain.to_string())
        .map_err(|e| format!("bad server name {}: {}", domain, e))?;
    let mut tls =
        tokio::time::timeout(Duration::from_secs(8), connector.connect(server_name, tcp)).await??;

    let request = format!(
        "GET / HTTP/1.1\r\n\
         Host: {host}\r\n\
         User-Agent: {ua}\r\n\
         Accept: text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,*/*;q=0.8\r\n\
         Accept-Language: fa,en;q=0.9\r\n\
         Accept-Encoding: gzip, deflate, br\r\n\
         Connection: close\r\n\
         \r\n",
        host = domain,
        ua = user_agent,
    );
    tls.write_all(request.as_bytes()).await?;

    // Read up to ~8 KB of response then drop. Reading some bytes matters
    // because DPI flow classifiers look at the bidirectional byte ratio,
    // not just the handshake.
    let mut buf = [0u8; 8192];
    let _ = tokio::time::timeout(Duration::from_secs(5), tls.read(&mut buf)).await;
    trace!("[decoy] visited {}", domain);
    Ok(())
}
