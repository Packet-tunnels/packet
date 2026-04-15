// phantom-bridge: Domestic relay for censored networks
//
// This is a lightweight TCP relay designed to run on a domestic VPS
// inside a censored country (e.g., Iran). It transparently forwards
// all TCP connections to the Phantom server running outside the country.
//
// Why this exists:
// In Iran's 2026 blockout, only traffic to domestic IPs passes through DPI.
// This bridge runs on a domestic VPS (whitelisted IP) and relays traffic
// to the foreign Phantom server, creating an invisible bridge.
//
// Architecture:
//   Client → Bridge (domestic VPS, port 80) → Phantom Server (GCP, foreign)
//   DPI sees: traffic to domestic IP on port 80 → ALLOWED
//
// The bridge is transparent — it doesn't understand the Phantom protocol.
// All encryption/decryption happens end-to-end between client and server.
// The bridge just copies bytes bidirectionally.
//
// For probe resistance, the Phantom server handles it:
// - Normal HTTP requests → piano lessons website
// - WebSocket upgrades → tunnel session
// - API POSTs → tunnel sync

use clap::Parser;
use tokio::io;
use tokio::net::{TcpListener, TcpStream};
use tracing::{error, info, warn};

#[derive(Parser)]
#[command(name = "phantom-bridge")]
#[command(about = "Phantom Tunnel Bridge — domestic relay for censored networks")]
struct Cli {
    /// Listen address (e.g., 0.0.0.0:80)
    #[arg(short, long, default_value = "0.0.0.0:80")]
    listen: String,

    /// Upstream phantom-server address (e.g., 35.222.22.49:80)
    #[arg(short, long)]
    upstream: String,

    /// Maximum concurrent connections
    #[arg(long, default_value = "1024")]
    max_connections: usize,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();

    info!("Phantom Bridge starting");
    info!("Listen: {}", cli.listen);
    info!("Upstream: {}", cli.upstream);
    info!("Max connections: {}", cli.max_connections);

    let listener = TcpListener::bind(&cli.listen)
        .await
        .expect("failed to bind listener");

    let active = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));

    loop {
        let (client, peer) = match listener.accept().await {
            Ok(c) => c,
            Err(e) => {
                error!("Accept error: {}", e);
                continue;
            }
        };

        let current = active.load(std::sync::atomic::Ordering::Relaxed);
        if current >= cli.max_connections {
            warn!("Max connections reached ({}), dropping {}", current, peer);
            drop(client);
            continue;
        }

        let upstream_addr = cli.upstream.clone();
        let active = active.clone();
        active.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        tokio::spawn(async move {
            info!("Bridge: {} → {}", peer, upstream_addr);

            match TcpStream::connect(&upstream_addr).await {
                Ok(server) => {
                    let (mut cr, mut cw) = client.into_split();
                    let (mut sr, mut sw) = server.into_split();

                    tokio::select! {
                        result = io::copy(&mut cr, &mut sw) => {
                            if let Err(e) = result {
                                // Normal — connection closed by client
                                let _ = e;
                            }
                        }
                        result = io::copy(&mut sr, &mut cw) => {
                            if let Err(e) = result {
                                let _ = e;
                            }
                        }
                    }
                }
                Err(e) => {
                    error!("Upstream connect failed: {} → {}", upstream_addr, e);
                }
            }

            active.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
            info!("Bridge: {} closed", peer);
        });
    }
}
