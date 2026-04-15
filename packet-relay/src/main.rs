// phantom-relay: Starlink Exit Node for Phantom Tunnel
//
// This binary runs on a device connected to unfiltered internet (Starlink).
// It connects OUTBOUND to the Phantom Server (GCP) and registers as a relay.
// The server then routes client traffic through this relay for internet access.
//
// Architecture:
//   [Mobile Client in Iran] → [GCP Phantom Server] → [This Relay on Starlink] → [Free Internet]
//
// Security:
//   - Relay connects outbound only — no public IP or port forwarding needed
//   - All traffic is encrypted with AES-256-GCM between server and relay
//   - The relay appears as a normal WebSocket connection to the server
//   - Starlink connection bypasses Iranian DPI entirely
//
// Usage:
//   phantom-relay --server http://35.222.22.49 --secret <shared-secret> --label "starlink-tehran-01"

use clap::Parser;
use futures_util::{SinkExt, StreamExt};
use phantom_proto::*;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::net::tcp::OwnedWriteHalf;
use tokio::sync::{mpsc, Mutex};
use tokio_tungstenite::tungstenite::Message;
use tracing::{error, info, warn};

#[derive(Parser)]
#[command(name = "phantom-relay")]
#[command(about = "Phantom Relay — Starlink exit node for Phantom Tunnel")]
struct Cli {
    /// Server URL (e.g., "http://35.222.22.49" or "http://piano-lessons.site")
    #[arg(short, long, env = "PHANTOM_SERVER")]
    server: String,

    /// Shared secret (must match server)
    #[arg(long, env = "PHANTOM_SECRET")]
    secret: String,

    /// Human-readable label for this relay node
    #[arg(short, long, default_value = "starlink-relay")]
    label: String,
}

/// State for managing active TCP connections made by this relay
struct RelayState {
    /// Active TCP writers for streams this relay has connected
    writers: HashMap<u32, OwnedWriteHalf>,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_target(false)
        .with_thread_ids(false)
        .init();

    let cli = Cli::parse();
    let key = derive_key(&cli.secret);

    info!("╔═══════════════════════════════════════════╗");
    info!("║  Phantom Relay v0.3.0 — Starlink Exit     ║");
    info!("╚═══════════════════════════════════════════╝");
    info!("Server: {}", cli.server);
    info!("Label:  {}", cli.label);
    info!("");

    let mut retry_count = 0u32;

    loop {
        info!("[RELAY] Connecting to server...");

        match run_relay_session(&cli.server, &cli.secret, &cli.label, &key).await {
            Ok(()) => {
                retry_count = 0;
                warn!("[RELAY] Session ended, reconnecting in 2s...");
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
            Err(e) => {
                retry_count += 1;
                let delay = std::cmp::min(2u64.pow(retry_count.min(5)), 60);
                error!("[RELAY] ❌ Error: {} | retry #{} in {}s", e, retry_count, delay);
                tokio::time::sleep(Duration::from_secs(delay)).await;
            }
        }
    }
}

async fn run_relay_session(
    server_url: &str,
    secret: &str,
    label: &str,
    key: &[u8; 32],
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let url = url::Url::parse(server_url)?;
    let host = url.host_str().unwrap_or("127.0.0.1");
    let port = url.port().unwrap_or(if url.scheme() == "https" { 443 } else { 80 });
    let connect_addr = format!("{}:{}", host, port);

    // Build the WebSocket upgrade request — looks like a teacher connecting to broadcast
    let ws_scheme = if url.scheme() == "https" { "wss" } else { "ws" };
    let ws_uri = format!("{}://{}/api/v1/lessons/broadcast", ws_scheme, host);
    let ws_key = tokio_tungstenite::tungstenite::handshake::client::generate_key();

    let request = http::Request::builder()
        .uri(&ws_uri)
        .header("Host", host)
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header("Sec-WebSocket-Key", &ws_key)
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/120.0.0.0 Safari/537.36")
        .body(())?;

    // TCP connect to server
    info!("[RELAY] TCP connecting to {}...", connect_addr);
    let tcp = TcpStream::connect(&connect_addr).await
        .map_err(|e| format!("TCP connect to {} failed: {}", connect_addr, e))?;
    let _ = tcp.set_nodelay(true);
    info!("[RELAY] ✓ TCP connected");

    // WebSocket handshake
    let (ws_stream, _response) = tokio_tungstenite::client_async(request, tcp).await
        .map_err(|e| format!("WebSocket handshake failed: {}", e))?;
    info!("[RELAY] ✓ WebSocket connected");

    let (mut ws_sender, mut ws_receiver) = ws_stream.split();

    // ── Authenticate as relay ──
    let ts = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let sig = sign_auth(secret, ts);
    let auth_json = serde_json::json!({
        "ts": ts,
        "sig": sig,
        "mode": "relay",
        "label": label,
    }).to_string();

    info!("[RELAY] Sending relay auth...");
    ws_sender.send(Message::Text(auth_json)).await?;

    // Wait for acceptance
    let resp = tokio::time::timeout(Duration::from_secs(10), ws_receiver.next())
        .await
        .map_err(|_| "Auth timeout")?
        .ok_or("Connection closed during auth")?
        .map_err(|e| format!("WS error: {}", e))?;

    match resp {
        Message::Text(text) => {
            let json: serde_json::Value = serde_json::from_str(&text)?;
            if let Some(err) = json.get("error") {
                return Err(format!("Server rejected relay: {}", err).into());
            }
            let relay_id = json["relay_id"].as_str().unwrap_or("?");
            info!("[RELAY] ✓ Accepted as relay node: {}...", &relay_id[..relay_id.len().min(16)]);
        }
        _ => return Err("Unexpected auth response".into()),
    }

    info!("═══════════════════════════════════════════");
    info!("  RELAY ACTIVE — routing traffic via Starlink");
    info!("═══════════════════════════════════════════");

    let key = *key;
    let relay_state = Arc::new(Mutex::new(RelayState {
        writers: HashMap::new(),
    }));

    // Channel for sending frames back to the server
    let (downstream_tx, mut downstream_rx) = mpsc::channel::<Frame>(4096);

    // ── Sender task: relay responses → encrypt → WS → server ──
    let sender_task = tokio::spawn(async move {
        loop {
            let mut frames = Vec::new();

            tokio::select! {
                result = downstream_rx.recv() => {
                    match result {
                        Some(frame) => frames.push(frame),
                        None => break,
                    }
                }
                _ = tokio::time::sleep(Duration::from_secs(25)) => {
                    if ws_sender.send(Message::Ping(vec![0x52, 0x4C])).await.is_err() {
                        break;
                    }
                    continue;
                }
            }

            while let Ok(f) = downstream_rx.try_recv() {
                frames.push(f);
                if frames.len() >= 256 { break; }
            }

            if !frames.is_empty() {
                let plaintext = encode_frames(&frames);
                let encrypted = encrypt(&key, &plaintext);
                if ws_sender.send(Message::Binary(encrypted)).await.is_err() {
                    break;
                }
            }
        }
    });

    // ── Receiver task: WS → decrypt → frames from server → make real TCP connections ──
    let rx_state = relay_state.clone();
    let receiver_task = tokio::spawn(async move {
        while let Some(msg) = ws_receiver.next().await {
            match msg {
                Ok(Message::Binary(data)) => {
                    match decrypt(&key, &data) {
                        Ok(plaintext) => {
                            match decode_frames(&plaintext) {
                                Ok(frames) => {
                                    for frame in frames {
                                        process_server_frame(
                                            frame,
                                            &rx_state,
                                            &downstream_tx,
                                        ).await;
                                    }
                                }
                                Err(e) => error!("[RELAY] Frame decode error: {}", e),
                            }
                        }
                        Err(e) => error!("[RELAY] Decrypt error: {}", e),
                    }
                }
                Ok(Message::Ping(_)) | Ok(Message::Pong(_)) => continue,
                Ok(Message::Close(reason)) => {
                    info!("[RELAY] Server closed connection: {:?}", reason);
                    break;
                }
                Err(e) => {
                    error!("[RELAY] WS error: {}", e);
                    break;
                }
                _ => {}
            }
        }
    });

    tokio::select! {
        _ = sender_task => {},
        _ = receiver_task => {},
    }

    Ok(())
}

/// Process a frame received from the server.
/// The server sends Connect/Data/Close frames that this relay executes
/// by making real TCP connections to the open internet.
async fn process_server_frame(
    frame: Frame,
    state: &Arc<Mutex<RelayState>>,
    downstream_tx: &mpsc::Sender<Frame>,
) {
    match frame.cmd {
        Cmd::Connect => {
            let addr = String::from_utf8_lossy(&frame.data).to_string();
            let stream_id = frame.stream_id;
            let tx = downstream_tx.clone();
            let state = state.clone();

            info!("[RELAY] Stream {} → connecting to {}", stream_id, addr);

            tokio::spawn(async move {
                match TcpStream::connect(&addr).await {
                    Ok(tcp) => {
                        let (mut read_half, write_half) = tcp.into_split();

                        {
                            let mut s = state.lock().await;
                            s.writers.insert(stream_id, write_half);
                        }

                        // Send ConnectOk back to server
                        let _ = tx.send(Frame::connect_ok(stream_id)).await;
                        info!("[RELAY] Stream {} ✓ connected to {}", stream_id, addr);

                        // Spawn reader: real internet TCP → downstream → server → client
                        let tx2 = tx.clone();
                        tokio::spawn(async move {
                            let mut buf = vec![0u8; 16384];
                            loop {
                                match read_half.read(&mut buf).await {
                                    Ok(0) => {
                                        let _ = tx2.send(Frame::close(stream_id)).await;
                                        break;
                                    }
                                    Ok(n) => {
                                        let _ = tx2.send(Frame::data(
                                            stream_id,
                                            buf[..n].to_vec(),
                                        )).await;
                                    }
                                    Err(e) => {
                                        error!("[RELAY] Stream {} read error: {}", stream_id, e);
                                        let _ = tx2.send(Frame::close(stream_id)).await;
                                        break;
                                    }
                                }
                            }
                        });
                    }
                    Err(e) => {
                        error!("[RELAY] Stream {} connect to {} failed: {}", stream_id, addr, e);
                        let _ = tx.send(Frame::connect_err(stream_id, &e.to_string())).await;
                    }
                }
            });
        }

        Cmd::Data => {
            let mut s = state.lock().await;
            if let Some(writer) = s.writers.get_mut(&frame.stream_id) {
                if let Err(e) = writer.write_all(&frame.data).await {
                    error!("[RELAY] Stream {} write error: {}", frame.stream_id, e);
                    s.writers.remove(&frame.stream_id);
                    let _ = downstream_tx.send(Frame::close(frame.stream_id)).await;
                }
            }
        }

        Cmd::Close => {
            info!("[RELAY] Stream {} closed by server", frame.stream_id);
            let mut s = state.lock().await;
            s.writers.remove(&frame.stream_id);
        }

        _ => {}
    }
}
