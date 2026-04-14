// transport.rs — Pluggable transport layer for Phantom Tunnel
//
// Supports multiple transport modes:
// - WebSocket: Persistent bidirectional connection (CDN-compatible, primary for Iran)
// - HTTP Polling: Original HTTP POST-based transport (fallback, backward compatible)
// - Auto: Try WebSocket first, fall back to HTTP if WebSocket fails
//
// CDN Mode Architecture (Iran bypass):
//   Client → ArvanCloud Edge (domestic IP:80) → CDN Forward → Phantom Server (GCP)
//   DPI sees: HTTP to domestic IP with domestic Host header → ALLOWED
//
// The WebSocket transport is essential for CDN passthrough because:
// 1. CDNs natively forward WebSocket connections to origin servers
// 2. WebSocket is bidirectional — no polling overhead
// 3. WebSocket upgrade looks like a normal web application feature
// 4. ArvanCloud specifically supports WebSocket forwarding

use crate::{dispatch_downstream, process_upstream_msg, stats, TunnelState, UpstreamMsg};
use futures_util::{SinkExt, StreamExt};
use phantom_proto::*;
use reqwest::Client;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, Mutex};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;
use tracing::{error, info, warn};

// ─── Transport Configuration ──────────────────────────────────

#[derive(Clone, Debug)]
pub enum TransportMode {
    Http,
    WebSocket,
    Auto,
}

#[derive(Clone, Debug)]
pub struct TransportConfig {
    pub server_url: String,
    pub secret: String,
    pub key: [u8; 32],
    pub mode: TransportMode,
    /// Custom Host header for CDN mode (e.g., "piano-lessons.site")
    pub host_header: Option<String>,
    /// CDN edge IP to connect to (e.g., "185.143.234.235:80")
    pub cdn_edge: Option<String>,
    /// Enable TLS ClientHello fragmentation
    pub fragment_enabled: bool,
    /// Fragment chunk size in bytes
    pub fragment_size: usize,
    /// Enable traffic padding
    pub padding_enabled: bool,
}

impl TransportConfig {
    /// Get the TCP address to connect to.
    /// CDN mode: connects to CDN edge IP.
    /// Direct mode: connects to server IP from URL.
    fn connect_addr(&self) -> String {
        if let Some(ref edge) = self.cdn_edge {
            if edge.contains(':') {
                edge.clone()
            } else {
                let port = if self.server_url.starts_with("https") {
                    443
                } else {
                    80
                };
                format!("{}:{}", edge, port)
            }
        } else {
            let url = url::Url::parse(&self.server_url)
                .unwrap_or_else(|_| url::Url::parse("http://127.0.0.1").unwrap());
            let host = url.host_str().unwrap_or("127.0.0.1");
            let port = url
                .port()
                .unwrap_or(if url.scheme() == "https" { 443 } else { 80 });
            format!("{}:{}", host, port)
        }
    }

    /// Get the Host header value.
    /// CDN mode: uses the override or domain from URL.
    /// Direct mode: uses domain from URL.
    fn host_value(&self) -> String {
        if let Some(ref host) = self.host_header {
            host.clone()
        } else {
            url::Url::parse(&self.server_url)
                .ok()
                .and_then(|u| u.host_str().map(|s| s.to_string()))
                .unwrap_or_else(|| "localhost".to_string())
        }
    }

    /// Whether the server URL uses HTTPS/WSS.
    fn is_tls(&self) -> bool {
        self.server_url.starts_with("https")
    }
}

// ─── Transport Runner ──────────────────────────────────────────

pub async fn run_transport(
    config: TransportConfig,
    upstream_rx: mpsc::Receiver<UpstreamMsg>,
    tunnel_state: Arc<Mutex<TunnelState>>,
) {
    match config.mode {
        TransportMode::WebSocket => {
            run_ws_loop(config, upstream_rx, tunnel_state).await;
        }
        TransportMode::Http => {
            run_http_loop(config, upstream_rx, tunnel_state).await;
        }
        TransportMode::Auto => {
            run_auto_loop(config, upstream_rx, tunnel_state).await;
        }
    }
}

// ═══════════════════════════════════════════════════════════════
// WebSocket Transport
// ═══════════════════════════════════════════════════════════════
//
// Primary transport for Iran bypass via CDN.
// Connects to ArvanCloud edge on port 80, sends WebSocket upgrade
// with Host: piano-lessons.site. CDN forwards to origin server.
// DPI sees normal domestic HTTP traffic.

async fn run_ws_loop(
    config: TransportConfig,
    mut upstream_rx: mpsc::Receiver<UpstreamMsg>,
    tunnel_state: Arc<Mutex<TunnelState>>,
) {
    let mut retry_count = 0u32;

    loop {
        let addr = config.connect_addr();
        let host = config.host_value();
        stats::set_state("connecting");
        info!(
            "[PHANTOM] WS connecting: addr={} host={} cdn_edge={:?} tls={}",
            addr,
            host,
            config.cdn_edge,
            config.is_tls()
        );

        match ws_session(&config, &mut upstream_rx, &tunnel_state).await {
            Ok(()) => {
                stats::mark_transport_disconnected(None);
                warn!("[PHANTOM] WebSocket session ended gracefully, reconnecting in 500ms...");
                retry_count = 0;
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            Err(e) => {
                let message = e.to_string();
                stats::mark_transport_disconnected(Some(message.clone()));
                retry_count += 1;
                let delay = std::cmp::min(2u64.pow(retry_count.min(5)), 30);
                error!(
                    "[PHANTOM] ❌ WS FAILED: {} | retry #{} in {}s | addr={} host={}",
                    message, retry_count, delay, addr, host
                );
                tokio::time::sleep(Duration::from_secs(delay)).await;
            }
        }
    }
}

/// Establish a single WebSocket session: connect, authenticate, relay.
async fn ws_session(
    config: &TransportConfig,
    upstream_rx: &mut mpsc::Receiver<UpstreamMsg>,
    tunnel_state: &Arc<Mutex<TunnelState>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let host = config.host_value();
    let connect_addr = config.connect_addr();

    // Build the WebSocket upgrade request
    // This is crafted to look like a normal browser WebSocket connection
    let ws_scheme = if config.is_tls() { "wss" } else { "ws" };
    let ws_uri = format!("{}://{}/api/v1/lessons/live", ws_scheme, host);
    let ws_key = tokio_tungstenite::tungstenite::handshake::client::generate_key();

    let request = http::Request::builder()
        .uri(&ws_uri)
        .header("Host", &host)
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header("Sec-WebSocket-Key", &ws_key)
        .header("User-Agent", random_user_agent())
        .header("Accept-Language", "en-US,en;q=0.9,fa;q=0.8")
        .header("Cache-Control", "no-cache")
        .header("Pragma", "no-cache")
        .body(())?;

    // Establish TCP connection to server (or CDN edge)
    info!("[PHANTOM] TCP connecting to {}...", connect_addr);
    let tcp = TcpStream::connect(&connect_addr).await.map_err(|e| {
        format!(
            "TCP connect to {} failed: {} (is the server/CDN reachable?)",
            connect_addr, e
        )
    })?;
    let _ = tcp.set_nodelay(true);
    info!("[PHANTOM] ✓ TCP connected to {}", connect_addr);

    // WebSocket handshake over the raw TCP stream
    // For CDN mode: the Host header tells ArvanCloud which origin to forward to
    info!(
        "[PHANTOM] WS handshake: upgrade request to {} via {}",
        ws_uri, connect_addr
    );
    let (ws_stream, response) = tokio_tungstenite::client_async(request, tcp)
        .await
        .map_err(|e| {
            format!(
                "WebSocket handshake to {} via {} failed: {} (CDN may block WS or Host mismatch)",
                host, connect_addr, e
            )
        })?;

    info!(
        "[PHANTOM] ✓ WebSocket connected — HTTP {} from {}",
        response.status(),
        host
    );

    // Authenticate and start relay
    ws_auth_and_relay(ws_stream, config, upstream_rx, tunnel_state).await
}

/// Authenticate over WebSocket, then run bidirectional relay.
/// Generic over stream type to support plain TCP, TLS, and fragmented TLS.
async fn ws_auth_and_relay<S>(
    ws_stream: WebSocketStream<S>,
    config: &TransportConfig,
    upstream_rx: &mut mpsc::Receiver<UpstreamMsg>,
    tunnel_state: &Arc<Mutex<TunnelState>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    let (mut ws_sender, mut ws_receiver) = ws_stream.split();
    let key = config.key;
    let ping_payload = vec![0x50, 0x54];

    // ── Authenticate ──
    let ts = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let sig = sign_auth(&config.secret, ts);
    let auth_json = serde_json::json!({"ts": ts, "sig": sig}).to_string();
    let auth_started_at = Instant::now();

    info!("[PHANTOM] Sending auth (ts={})...", ts);
    ws_sender
        .send(Message::Text(auth_json))
        .await
        .map_err(|e| {
            format!(
                "Failed to send auth message: {} (connection died before auth)",
                e
            )
        })?;

    // Wait for auth response
    info!("[PHANTOM] Waiting for auth response (10s timeout)...");
    let auth_resp = tokio::time::timeout(Duration::from_secs(10), ws_receiver.next())
        .await
        .map_err(|_| "Auth timeout: server did not respond within 10s (check secret matches & server is running)")?
        .ok_or("Connection closed during auth: server dropped connection (check firewall/CDN config)")?
        .map_err(|e| format!("WS error during auth: {} (possible CDN interference)", e))?;

    match auth_resp {
        Message::Text(text) => {
            let json: serde_json::Value = serde_json::from_str(&text).map_err(|e| {
                format!(
                    "Auth response not JSON: {} — raw response: '{}'",
                    e,
                    &text[..text.len().min(200)]
                )
            })?;
            if let Some(err) = json.get("error") {
                return Err(format!(
                    "❌ Auth REJECTED by server: {} (check secret matches & clock sync)",
                    err
                )
                .into());
            }
            let token = json["token"].as_str().unwrap_or("unknown");
            info!(
                "[PHANTOM] ✓ Authenticated — session: {}...",
                &token[..token.len().min(16)]
            );
            stats::note_ping(auth_started_at.elapsed().as_millis().min(u32::MAX as u128) as u32);
        }
        other => {
            return Err(format!(
                "Unexpected auth response type: {:?} (expected Text JSON)",
                other
            )
            .into());
        }
    }

    // ── Bidirectional Relay ──
    // Uses select! to concurrently handle upstream (SOCKS5→WS) and
    // downstream (WS→SOCKS5) without spawning tasks. This keeps
    // upstream_rx borrowed (not moved) for reconnection support.
    info!("[PHANTOM] ✓ TUNNEL ACTIVE — relay is live");
    stats::mark_transport_connected();

    let mut ping_interval = tokio::time::interval(Duration::from_secs(15));
    ping_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    ping_interval.tick().await;
    let mut last_ping_sent_at: Option<Instant> = None;

    loop {
        tokio::select! {
            _ = ping_interval.tick() => {
                last_ping_sent_at = Some(Instant::now());
                ws_sender.send(Message::Ping(ping_payload.clone())).await
                    .map_err(|e| format!("WS ping failed: {} (connection to CDN/server dropped)", e))?;
            }

            // Upstream: SOCKS5 handlers → encrypt → WebSocket → CDN → server
            msg = upstream_rx.recv() => {
                match msg {
                    Some(msg) => {
                        let mut frames = Vec::new();
                        process_upstream_msg(msg, &mut frames, tunnel_state).await;

                        // Drain any queued messages for batching
                        while let Ok(msg) = upstream_rx.try_recv() {
                            process_upstream_msg(msg, &mut frames, tunnel_state).await;
                        }

                        if !frames.is_empty() {
                            let plaintext = encode_frames(&frames);
                            let encrypted = encrypt(&key, &plaintext);
                            ws_sender.send(Message::Binary(encrypted)).await
                                .map_err(|e| format!("WS send failed: {} (connection to CDN/server dropped)", e))?;
                        }
                    }
                    None => {
                        error!("[PHANTOM] Upstream channel closed — no more SOCKS5 streams");
                        return Ok(());
                    }
                }
            }

            // Downstream: server → CDN → WebSocket → decrypt → SOCKS5 handlers
            msg = ws_receiver.next() => {
                match msg {
                    Some(Ok(Message::Binary(data))) => {
                        match decrypt(&key, &data) {
                            Ok(plaintext) => {
                                match decode_frames(&plaintext) {
                                    Ok(frames) => {
                                        dispatch_downstream(frames, tunnel_state).await;
                                    }
                                    Err(e) => {
                                        error!("[PHANTOM] Frame decode error: {} ({}B data)", e, plaintext.len());
                                    }
                                }
                            }
                            Err(e) => {
                                error!("[PHANTOM] Decrypt failed: {} ({}B data — key mismatch or data corrupt)", e, data.len());
                            }
                        }
                    }
                    Some(Ok(Message::Ping(_))) => continue,
                    Some(Ok(Message::Pong(payload))) => {
                        if payload == ping_payload {
                            if let Some(sent_at) = last_ping_sent_at.take() {
                                stats::note_ping(sent_at.elapsed().as_millis().min(u32::MAX as u128) as u32);
                            }
                        }
                        continue;
                    }
                    Some(Ok(Message::Close(reason))) => {
                        info!("[PHANTOM] WebSocket closed by server: {:?}", reason);
                        return Ok(());
                    }
                    Some(Err(e)) => {
                        return Err(format!("WS receive error: {} (CDN timeout or network drop)", e).into());
                    }
                    None => {
                        return Err("WebSocket stream ended unexpectedly (CDN or server closed connection)".into());
                    }
                    _ => continue,
                }
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════
// HTTP Polling Transport (backward compatible)
// ═══════════════════════════════════════════════════════════════
//
// Original transport — makes HTTP POST requests to /api/v1/lessons/sync.
// Works for direct connections (no CDN needed).
// Falls back to this if WebSocket is not available.

async fn run_http_loop(
    config: TransportConfig,
    mut upstream_rx: mpsc::Receiver<UpstreamMsg>,
    tunnel_state: Arc<Mutex<TunnelState>>,
) {
    // Build HTTP client with connection pooling
    let http_client = Client::builder()
        .pool_max_idle_per_host(4)
        .timeout(Duration::from_secs(30))
        .danger_accept_invalid_certs(true) // for CDN flexibility
        .build()
        .expect("failed to build HTTP client");

    // Authenticate with server
    info!("[PHANTOM] HTTP authenticating to {}...", config.server_url);
    let token = loop {
        match http_authenticate(&http_client, &config).await {
            Ok(t) => break t,
            Err(e) => {
                error!(
                    "[PHANTOM] ❌ HTTP auth failed: {} | server={} | retrying in 5s...",
                    e, config.server_url
                );
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        }
    };

    info!("HTTP authenticated: {}...", &token[..16]);
    stats::mark_transport_connected();

    let sync_url = format!("{}/api/v1/lessons/sync", config.server_url);
    let key = config.key;
    let mut _last_data = Instant::now();
    let mut consecutive_empty: u32 = 0;
    let poll_active = Duration::from_millis(50);
    let poll_idle = Duration::from_millis(200);

    loop {
        // Collect pending upstream messages
        let mut upstream_frames = Vec::new();
        let poll_interval = if consecutive_empty > 10 {
            poll_idle
        } else {
            poll_active
        };

        match tokio::time::timeout(poll_interval, upstream_rx.recv()).await {
            Ok(Some(msg)) => {
                process_upstream_msg(msg, &mut upstream_frames, &tunnel_state).await;
            }
            Ok(None) => {
                error!("Upstream channel closed, exiting HTTP poller");
                return;
            }
            Err(_) => {} // timeout, just poll
        }

        // Drain remaining messages
        while let Ok(msg) = upstream_rx.try_recv() {
            process_upstream_msg(msg, &mut upstream_frames, &tunnel_state).await;
        }

        // Encode, encrypt, and send
        let plaintext = encode_frames(&upstream_frames);
        let encrypted = encrypt(&key, &plaintext);
        let encoded = b64_encode(&encrypted);

        let req_body = SyncRequest {
            t: token.clone(),
            d: encoded,
        };

        match http_client.post(&sync_url).json(&req_body).send().await {
            Ok(resp) => {
                let status = resp.status();
                match resp.json::<SyncResponse>().await {
                    Ok(sync_resp) => {
                        stats::mark_transport_connected();
                        match b64_decode(&sync_resp.d) {
                            Ok(encrypted) => match decrypt(&key, &encrypted) {
                                Ok(plaintext) => match decode_frames(&plaintext) {
                                    Ok(frames) => {
                                        if !frames.is_empty() {
                                            consecutive_empty = 0;
                                            _last_data = Instant::now();
                                        } else {
                                            consecutive_empty = consecutive_empty.saturating_add(1);
                                        }
                                        dispatch_downstream(frames, &tunnel_state).await;
                                    }
                                    Err(e) => error!("[PHANTOM] Frame decode error: {}", e),
                                },
                                Err(e) => error!("[PHANTOM] Decrypt error: {} (key mismatch?)", e),
                            },
                            Err(e) => error!("[PHANTOM] Base64 decode error: {}", e),
                        }
                    }
                    Err(e) => {
                        error!(
                            "[PHANTOM] Sync response parse error: {} (HTTP {})",
                            e, status
                        );
                    }
                }
            }
            Err(e) => {
                stats::set_state("reconnecting");
                stats::set_error(e.to_string());
                error!("[PHANTOM] ❌ Sync request failed: {} | url={}", e, sync_url);
                tokio::time::sleep(Duration::from_secs(2)).await;
                consecutive_empty = 0;
            }
        }
    }
}

/// HTTP authentication — POST to /api/v1/auth/login
async fn http_authenticate(client: &Client, config: &TransportConfig) -> Result<String, String> {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let sig = sign_auth(&config.secret, ts);

    let url = format!("{}/api/v1/auth/login", config.server_url);
    let body = AuthRequest { ts, sig };

    let resp = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("request failed: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("auth failed: status {}", resp.status()));
    }

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("bad response: {}", e))?;

    json["token"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| "no token in response".to_string())
}

// ═══════════════════════════════════════════════════════════════
// Auto Transport (WebSocket → HTTP fallback)
// ═══════════════════════════════════════════════════════════════

async fn run_auto_loop(
    config: TransportConfig,
    mut upstream_rx: mpsc::Receiver<UpstreamMsg>,
    tunnel_state: Arc<Mutex<TunnelState>>,
) {
    let mut ws_failures = 0u32;

    loop {
        if ws_failures < 5 {
            stats::set_state("connecting");
            info!(
                "[PHANTOM] Auto mode: trying WebSocket (attempt {}/5) to {}",
                ws_failures + 1,
                config.connect_addr()
            );
            match ws_session(&config, &mut upstream_rx, &tunnel_state).await {
                Ok(()) => {
                    stats::mark_transport_disconnected(None);
                    ws_failures = 0;
                    info!("[PHANTOM] WS session ended, reconnecting...");
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    continue;
                }
                Err(e) => {
                    stats::mark_transport_disconnected(Some(e.to_string()));
                    ws_failures += 1;
                    error!("[PHANTOM] ❌ Auto WS failed #{}/5: {}", ws_failures, e);
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
            }
        } else {
            warn!("[PHANTOM] WebSocket failed 5x, switching to HTTP polling permanently");
            run_http_loop(config, upstream_rx, tunnel_state).await;
            return;
        }
    }
}

// ─── Helpers ───────────────────────────────────────────────────

/// Rotate through common User-Agent strings to avoid fingerprinting.
fn random_user_agent() -> &'static str {
    static UAS: &[&str] = &[
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:121.0) Gecko/20100101 Firefox/121.0",
        "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
        "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Mobile/15E148 Safari/604.1",
    ];
    let idx = std::process::id() as usize % UAS.len();
    UAS[idx]
}
