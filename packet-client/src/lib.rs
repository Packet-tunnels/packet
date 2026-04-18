// phantom-client: Covert tunnel client with CDN bypass support
//
// This client does four things:
// 1. Opens a local SOCKS5 proxy (default :1080) for apps to connect through
// 2. Multiplexes all SOCKS5 streams into encrypted frames
// 3. Sends frames via WebSocket (CDN-compatible) or HTTP POST (fallback)
// 4. Supports CDN mode for bypassing internet blockouts (Iran, etc.)
//
// CDN bypass architecture:
//   Browser → SOCKS5 :1080 → Phantom Client → WebSocket → CDN Edge → Phantom Server → Internet
//   DPI sees: HTTP WebSocket to domestic CDN IP with domestic domain → ALLOWED

#[cfg(target_os = "android")]
pub(crate) mod android_tun;
pub mod ffi;
pub(crate) mod fragment;
pub(crate) mod transport;

use lazy_static::lazy_static;
use phantom_proto::*;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::sync::{watch, Mutex};
use tracing::{error, info, warn};
use url::Url;

// Re-export for external use
pub use transport::TransportMode;

// ─── Client Configuration ──────────────────────────────────────

#[derive(Clone, Debug)]
pub struct ClientConfig {
    /// Server URL (e.g., "http://piano-lessons.site" or "http://35.222.22.49")
    pub server_url: String,
    /// Shared secret (must match server)
    pub secret: String,
    /// Local SOCKS5 listen address (e.g., "127.0.0.1:1080")
    pub listen: String,
    /// Transport mode: WebSocket, Http, or Auto
    pub transport: TransportMode,
    /// CDN edge IP:port to connect to (e.g., "185.143.234.235:80")
    /// When set, client connects to this IP instead of the server URL's host
    pub cdn_edge: Option<String>,
    /// Custom Host header (e.g., "piano-lessons.site")
    /// Used with CDN mode to tell the CDN which origin to forward to
    pub host_override: Option<String>,
    /// Enable TLS ClientHello fragmentation (for HTTPS connections)
    pub fragment: bool,
    /// Fragment chunk size in bytes (default: 40)
    pub fragment_size: usize,
    /// Enable traffic padding to prevent size-based fingerprinting
    pub padding: bool,
    /// Custom SNI for TLS handshake (for DPI bypass)
    /// When set, TLS ClientHello uses this SNI instead of the real host
    pub sni_override: Option<String>,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            server_url: String::new(),
            secret: String::new(),
            listen: "127.0.0.1:1080".to_string(),
            transport: TransportMode::Auto,
            cdn_edge: None,
            host_override: None,
            fragment: false,
            fragment_size: 40,
            padding: true,
            sni_override: None,
        }
    }
}

// ─── Runtime Stats ─────────────────────────────────────────────

#[derive(Clone, Debug, Default, Serialize)]
pub struct RuntimeStatsSnapshot {
    pub state: String,
    pub transport: String,
    pub server_host: String,
    pub cdn_edge: Option<String>,
    pub listen_port: Option<u16>,
    pub bytes_up: u64,
    pub bytes_down: u64,
    pub active_streams: u32,
    pub total_streams: u64,
    pub connected_since: Option<u64>,
    pub last_ping_ms: Option<u32>,
    pub last_error: Option<String>,
    pub tunnel_active: bool,
}

lazy_static! {
    static ref RUNTIME_STATS: StdMutex<RuntimeStatsSnapshot> =
        StdMutex::new(RuntimeStatsSnapshot::default());
}

fn runtime_transport_label(mode: &TransportMode) -> &'static str {
    match mode {
        TransportMode::Http => "HTTP",
        TransportMode::WebSocket => "WebSocket",
        TransportMode::Auto => "Auto",
    }
}

fn runtime_server_host(server_url: &str) -> String {
    Url::parse(server_url)
        .ok()
        .and_then(|url| url.host_str().map(|value| value.to_string()))
        .unwrap_or_else(|| {
            server_url
                .trim()
                .trim_start_matches("http://")
                .trim_start_matches("https://")
                .split('/')
                .next()
                .unwrap_or_default()
                .split(':')
                .next()
                .unwrap_or_default()
                .to_string()
        })
}

fn runtime_now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

fn runtime_listen_port(listen_addr: &str) -> Option<u16> {
    listen_addr
        .rsplit(':')
        .next()
        .and_then(|value| value.parse::<u16>().ok())
}

pub fn reset_runtime_stats(config: &ClientConfig) {
    if let Ok(mut snapshot) = RUNTIME_STATS.lock() {
        *snapshot = RuntimeStatsSnapshot {
            state: "starting".to_string(),
            transport: runtime_transport_label(&config.transport).to_string(),
            server_host: runtime_server_host(&config.server_url),
            cdn_edge: config.cdn_edge.clone(),
            listen_port: runtime_listen_port(&config.listen),
            bytes_up: 0,
            bytes_down: 0,
            active_streams: 0,
            total_streams: 0,
            connected_since: None,
            last_ping_ms: None,
            last_error: None,
            tunnel_active: false,
        };
    }
}

pub fn runtime_stats_json() -> Option<String> {
    let snapshot = RUNTIME_STATS.lock().ok()?.clone();
    serde_json::to_string(&snapshot).ok()
}

pub fn clear_runtime_stats() {
    if let Ok(mut snapshot) = RUNTIME_STATS.lock() {
        *snapshot = RuntimeStatsSnapshot::default();
    }
}

pub fn set_runtime_state(state: &str) {
    if let Ok(mut snapshot) = RUNTIME_STATS.lock() {
        snapshot.state = state.to_string();
        if state != "connected" {
            snapshot.tunnel_active = false;
        }
    }
}

pub fn set_runtime_connected(ping_ms: Option<u32>) {
    if let Ok(mut snapshot) = RUNTIME_STATS.lock() {
        snapshot.state = "connected".to_string();
        snapshot.tunnel_active = true;
        snapshot.last_error = None;
        if snapshot.connected_since.is_none() {
            snapshot.connected_since = Some(runtime_now_secs());
        }
        if let Some(ping_ms) = ping_ms {
            snapshot.last_ping_ms = Some(ping_ms);
        }
    }
}

pub fn set_runtime_last_error(message: impl Into<String>) {
    if let Ok(mut snapshot) = RUNTIME_STATS.lock() {
        snapshot.last_error = Some(message.into());
    }
}

pub fn add_runtime_bytes_up(bytes: u64) {
    if let Ok(mut snapshot) = RUNTIME_STATS.lock() {
        snapshot.bytes_up = snapshot.bytes_up.saturating_add(bytes);
    }
}

pub fn add_runtime_bytes_down(bytes: u64) {
    if let Ok(mut snapshot) = RUNTIME_STATS.lock() {
        snapshot.bytes_down = snapshot.bytes_down.saturating_add(bytes);
    }
}

pub fn increment_runtime_total_streams() {
    if let Ok(mut snapshot) = RUNTIME_STATS.lock() {
        snapshot.total_streams = snapshot.total_streams.saturating_add(1);
    }
}

pub fn increment_runtime_active_streams() {
    if let Ok(mut snapshot) = RUNTIME_STATS.lock() {
        snapshot.active_streams = snapshot.active_streams.saturating_add(1);
    }
}

pub fn decrement_runtime_active_streams() {
    if let Ok(mut snapshot) = RUNTIME_STATS.lock() {
        snapshot.active_streams = snapshot.active_streams.saturating_sub(1);
    }
}

pub fn set_runtime_ping(ping_ms: u32) {
    if let Ok(mut snapshot) = RUNTIME_STATS.lock() {
        snapshot.last_ping_ms = Some(ping_ms);
    }
}

// ─── Internal Types (shared with transport module) ─────────────

pub(crate) enum UpstreamMsg {
    Connect {
        stream_id: u32,
        addr: String,
        reply: tokio::sync::oneshot::Sender<bool>,
    },
    Data {
        stream_id: u32,
        data: Vec<u8>,
    },
    Close {
        stream_id: u32,
    },
}

pub(crate) struct TunnelState {
    /// Per-stream downstream data channels
    pub downstream_txs: HashMap<u32, mpsc::Sender<Vec<u8>>>,
    /// Pending connect reply channels
    pub connect_replies: HashMap<u32, tokio::sync::oneshot::Sender<bool>>,
}

// ─── Entry Points ──────────────────────────────────────────────

/// Start the client with minimal configuration (backward compatible).
pub async fn start_client(server_url: String, secret: String, listen: String) {
    start_client_with_config(ClientConfig {
        server_url,
        secret,
        listen,
        transport: TransportMode::Auto,
        ..Default::default()
    })
    .await;
}

/// Bind the SOCKS5 listener synchronously with SO_REUSEADDR.
/// Called from FFI layer BEFORE spawning the async runtime thread,
/// so that port conflicts are detected immediately.
pub fn bind_socks_listener(listen_addr: &str) -> Result<std::net::TcpListener, String> {
    let addr: std::net::SocketAddr = listen_addr
        .parse()
        .map_err(|e| format!("Invalid listen address {}: {}", listen_addr, e))?;

    let domain = if addr.is_ipv4() {
        socket2::Domain::IPV4
    } else {
        socket2::Domain::IPV6
    };

    let sock = socket2::Socket::new(domain, socket2::Type::STREAM, Some(socket2::Protocol::TCP))
        .map_err(|e| format!("Failed to create socket: {}", e))?;

    sock.set_reuse_address(true)
        .map_err(|e| format!("Failed to set SO_REUSEADDR: {}", e))?;

    #[cfg(any(target_os = "macos", target_os = "ios"))]
    sock.set_reuse_port(true)
        .map_err(|e| format!("Failed to set SO_REUSEPORT: {}", e))?;

    sock.bind(&addr.into())
        .map_err(|e| format!("Failed to bind on {}: {}", listen_addr, e))?;

    sock.listen(1024)
        .map_err(|e| format!("Failed to listen on {}: {}", listen_addr, e))?;

    sock.set_nonblocking(true)
        .map_err(|e| format!("Failed to set non-blocking: {}", e))?;

    Ok(sock.into())
}

/// Start the client with a pre-bound listener (called from FFI).
pub async fn start_client_with_listener(
    config: ClientConfig,
    std_listener: std::net::TcpListener,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    reset_runtime_stats(&config);

    let key = derive_key(&config.secret);
    let server_url = config.server_url.trim_end_matches('/').to_string();

    info!("[PHANTOM] ═══════════════════════════════════════");
    info!("[PHANTOM] Phantom Client v0.2.0 starting");
    info!("[PHANTOM] Server: {}", server_url);
    info!("[PHANTOM] Transport: {:?}", config.transport);
    info!("[PHANTOM] SOCKS5 proxy: {}", config.listen);
    info!(
        "[PHANTOM] Padding: {}",
        if config.padding { "ON" } else { "OFF" }
    );
    if let Some(ref edge) = config.cdn_edge {
        info!("[PHANTOM] CDN edge: {}", edge);
    }
    if let Some(ref host) = config.host_override {
        info!("[PHANTOM] Host override: {}", host);
    }
    if config.fragment {
        info!(
            "[PHANTOM] TLS fragment: ON ({}B chunks)",
            config.fragment_size
        );
    }
    info!("[PHANTOM] ═══════════════════════════════════════");

    // Convert std listener to tokio listener
    let listener = match TcpListener::from_std(std_listener) {
        Ok(l) => l,
        Err(e) => {
            error!("[PHANTOM] ❌ Failed to convert listener to async: {}", e);
            return;
        }
    };

    info!("[PHANTOM] ✓ SOCKS5 proxy listening on {}", config.listen);

    // Channels for SOCKS5 handler ↔ transport communication
    let (upstream_tx, upstream_rx) = mpsc::channel::<UpstreamMsg>(4096);

    let tunnel_state = Arc::new(Mutex::new(TunnelState {
        downstream_txs: HashMap::new(),
        connect_replies: HashMap::new(),
    }));

    // Build transport configuration
    let transport_config = transport::TransportConfig {
        server_url: server_url.clone(),
        secret: config.secret.clone(),
        key,
        mode: config.transport.clone(),
        host_header: config.host_override.clone(),
        cdn_edge: config.cdn_edge.clone(),
        sni_override: config.sni_override.clone(),
        fragment_enabled: config.fragment,
        fragment_size: config.fragment_size,
    };

    // Spawn the transport loop (WebSocket, HTTP, or Auto)
    // NOTE: Transport starts AFTER SOCKS5 is confirmed bound.
    info!("[PHANTOM] Launching transport...");
    let transport_state = tunnel_state.clone();
    tokio::spawn(async move {
        transport::run_transport(transport_config, upstream_rx, transport_state).await;
    });

    // Accept SOCKS5 connections
    let next_stream_id = Arc::new(std::sync::atomic::AtomicU32::new(1));

    loop {
        tokio::select! {
            accept_result = listener.accept() => {
                let (socket, peer) = match accept_result {
                    Ok(value) => value,
                    Err(error) => {
                        error!("[PHANTOM] ❌ SOCKS5 accept failed: {}", error);
                        break;
                    }
                };
                info!("SOCKS5 connection from {}", peer);

                let stream_id = next_stream_id.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let tx = upstream_tx.clone();
                let state = tunnel_state.clone();

                tokio::spawn(async move {
                    if let Err(e) = handle_socks5(socket, stream_id, tx, state).await {
                        warn!("[PHANTOM] SOCKS5 stream {} error: {}", stream_id, e);
                    }
                });
            }
            changed = shutdown_rx.changed() => {
                match changed {
                    Ok(()) => {
                        if *shutdown_rx.borrow() {
                            info!("[PHANTOM] Shutdown requested for local client");
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        }
    }

    info!("[PHANTOM] Client listener stopped");
}

/// Start the client with full configuration (binds port internally — for CLI use).
pub async fn start_client_with_config(config: ClientConfig) {
    match bind_socks_listener(&config.listen) {
        Ok(listener) => {
            let (_shutdown_tx, shutdown_rx) = watch::channel(false);
            start_client_with_listener(config, listener, shutdown_rx).await;
        }
        Err(e) => {
            error!("[PHANTOM] ❌ {}", e);
        }
    }
}

// ─── Shared Helpers (used by transport module) ─────────────────

/// Convert an upstream message into protocol frames.
pub(crate) async fn process_upstream_msg(
    msg: UpstreamMsg,
    frames: &mut Vec<Frame>,
    tunnel_state: &Arc<Mutex<TunnelState>>,
) {
    match msg {
        UpstreamMsg::Connect {
            stream_id,
            addr,
            reply,
        } => {
            increment_runtime_total_streams();
            frames.push(Frame::connect(stream_id, &addr));
            // Store the reply sender for when we get CONNECT_OK/ERR back
            tunnel_state
                .lock()
                .await
                .connect_replies
                .insert(stream_id, reply);
        }
        UpstreamMsg::Data { stream_id, data } => {
            add_runtime_bytes_up(data.len() as u64);
            // Split large data into multiple frames (max 65535 bytes per frame)
            for chunk in data.chunks(32768) {
                frames.push(Frame::data(stream_id, chunk.to_vec()));
            }
        }
        UpstreamMsg::Close { stream_id } => {
            decrement_runtime_active_streams();
            frames.push(Frame::close(stream_id));
        }
    }
}

/// Dispatch downstream frames to the appropriate SOCKS5 handlers.
pub(crate) async fn dispatch_downstream(
    frames: Vec<Frame>,
    tunnel_state: &Arc<Mutex<TunnelState>>,
) {
    let mut state = tunnel_state.lock().await;

    for frame in frames {
        match frame.cmd {
            Cmd::ConnectOk => {
                increment_runtime_active_streams();
                if let Some(reply) = state.connect_replies.remove(&frame.stream_id) {
                    let _ = reply.send(true);
                }
            }
            Cmd::ConnectErr => {
                set_runtime_last_error(String::from_utf8_lossy(&frame.data).to_string());
                warn!(
                    "Stream {} connect error: {}",
                    frame.stream_id,
                    String::from_utf8_lossy(&frame.data)
                );
                if let Some(reply) = state.connect_replies.remove(&frame.stream_id) {
                    let _ = reply.send(false);
                }
            }
            Cmd::Data => {
                add_runtime_bytes_down(frame.data.len() as u64);
                if let Some(tx) = state.downstream_txs.get(&frame.stream_id) {
                    if tx.send(frame.data).await.is_err() {
                        state.downstream_txs.remove(&frame.stream_id);
                    }
                }
            }
            Cmd::Close => {
                decrement_runtime_active_streams();
                if let Some(tx) = state.downstream_txs.remove(&frame.stream_id) {
                    let _ = tx.send(vec![]).await; // empty = close signal
                }
            }
            _ => {}
        }
    }
}

// ─── SOCKS5 Handler ────────────────────────────────────────────
// Minimal SOCKS5 implementation (CONNECT only, no-auth)

async fn handle_socks5(
    mut socket: tokio::net::TcpStream,
    stream_id: u32,
    upstream_tx: mpsc::Sender<UpstreamMsg>,
    tunnel_state: Arc<Mutex<TunnelState>>,
) -> Result<(), Box<dyn std::error::Error>> {
    // ── Step 1: SOCKS5 greeting ──
    let mut buf = [0u8; 258];
    let n = socket.read(&mut buf).await?;
    if n < 2 || buf[0] != 0x05 {
        return Err("not SOCKS5".into());
    }
    // Respond: version 5, no auth required
    socket.write_all(&[0x05, 0x00]).await?;

    // ── Step 2: SOCKS5 CONNECT request ──
    let n = socket.read(&mut buf).await?;
    if n < 4 || buf[0] != 0x05 {
        return Err("invalid SOCKS5 request".into());
    }

    if buf[1] != 0x01 {
        let command = match buf[1] {
            0x02 => "BIND",
            0x03 => "UDP ASSOCIATE",
            0x05 => "UDP FORWARD",
            _ => "UNKNOWN",
        };
        info!(
            "Stream {} unsupported SOCKS5 command 0x{:02x} ({})",
            stream_id, buf[1], command
        );
        socket
            .write_all(&[0x05, 0x07, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
            .await?;
        return Ok(());
    }

    let addr = match buf[3] {
        // IPv4
        0x01 => {
            if n < 10 {
                return Err("truncated IPv4".into());
            }
            let ip = format!("{}.{}.{}.{}", buf[4], buf[5], buf[6], buf[7]);
            let port = u16::from_be_bytes([buf[8], buf[9]]);
            format!("{}:{}", ip, port)
        }
        // Domain name
        0x03 => {
            let dlen = buf[4] as usize;
            if n < 5 + dlen + 2 {
                return Err("truncated domain".into());
            }
            let domain = std::str::from_utf8(&buf[5..5 + dlen])?;
            let port = u16::from_be_bytes([buf[5 + dlen], buf[5 + dlen + 1]]);
            format!("{}:{}", domain, port)
        }
        // IPv6
        0x04 => {
            if n < 22 {
                return Err("truncated IPv6".into());
            }
            let mut parts = Vec::new();
            for i in 0..8 {
                let w = u16::from_be_bytes([buf[4 + i * 2], buf[5 + i * 2]]);
                parts.push(format!("{:x}", w));
            }
            let port = u16::from_be_bytes([buf[20], buf[21]]);
            format!("[{}]:{}", parts.join(":"), port)
        }
        _ => return Err("unknown ATYP".into()),
    };

    info!("Stream {} CONNECT to {}", stream_id, addr);

    // Create downstream channel for this stream
    let (down_tx, mut down_rx) = mpsc::channel::<Vec<u8>>(512);

    // Register downstream channel
    {
        let mut state = tunnel_state.lock().await;
        state.downstream_txs.insert(stream_id, down_tx);
    }

    // Send CONNECT request via upstream channel and wait for reply
    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();

    upstream_tx
        .send(UpstreamMsg::Connect {
            stream_id,
            addr: addr.clone(),
            reply: reply_tx,
        })
        .await
        .map_err(|_| "upstream channel closed")?;

    // Wait for connect result (with timeout)
    let connected = tokio::time::timeout(Duration::from_secs(15), reply_rx)
        .await
        .map_err(|_| {
            format!(
                "connect timeout: {} did not respond in 15s (tunnel may be down)",
                addr
            )
        })?
        .map_err(|_| {
            format!(
                "connect reply dropped for {} (transport reconnecting?)",
                addr
            )
        })?;

    if !connected {
        // Send SOCKS5 failure response
        error!(
            "[PHANTOM] Stream {} remote connect to {} FAILED",
            stream_id, addr
        );
        socket
            .write_all(&[0x05, 0x05, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
            .await?;
        return Err(format!("remote connect to {} failed", addr).into());
    }

    // Send SOCKS5 success response
    socket
        .write_all(&[0x05, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
        .await?;

    info!("Stream {} connected to {}", stream_id, addr);

    // ── Step 3: Bidirectional data relay ──
    let (mut sock_read, mut sock_write) = socket.into_split();

    // Task: socket → upstream (read from browser, send to tunnel)
    let up_tx = upstream_tx.clone();
    let sid = stream_id;
    let reader_task = tokio::spawn(async move {
        let mut buf = vec![0u8; 16384];
        loop {
            match sock_read.read(&mut buf).await {
                Ok(0) => {
                    let _ = up_tx.send(UpstreamMsg::Close { stream_id: sid }).await;
                    break;
                }
                Ok(n) => {
                    let _ = up_tx
                        .send(UpstreamMsg::Data {
                            stream_id: sid,
                            data: buf[..n].to_vec(),
                        })
                        .await;
                }
                Err(_) => {
                    let _ = up_tx.send(UpstreamMsg::Close { stream_id: sid }).await;
                    break;
                }
            }
        }
    });

    // Task: downstream → socket (receive from tunnel, write to browser)
    let writer_task = tokio::spawn(async move {
        while let Some(data) = down_rx.recv().await {
            if data.is_empty() {
                break; // Close signal
            }
            if sock_write.write_all(&data).await.is_err() {
                break;
            }
        }
    });

    // Wait for either direction to finish
    tokio::select! {
        _ = reader_task => {},
        _ = writer_task => {},
    }

    // Cleanup
    {
        let mut state = tunnel_state.lock().await;
        state.downstream_txs.remove(&stream_id);
    }

    info!("Stream {} closed", stream_id);
    Ok(())
}
