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

pub mod ffi;
pub(crate) mod fragment;
pub(crate) mod transport;

use phantom_proto::*;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::sync::Mutex;
use tracing::{error, info, warn};

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
        }
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

/// Start the client with full configuration.
pub async fn start_client_with_config(config: ClientConfig) {
    let key = derive_key(&config.secret);
    let server_url = config.server_url.trim_end_matches('/').to_string();

    info!("[PHANTOM] ═══════════════════════════════════════");
    info!("[PHANTOM] Phantom Client v0.2.0 starting");
    info!("[PHANTOM] Server: {}", server_url);
    info!("[PHANTOM] Transport: {:?}", config.transport);
    info!("[PHANTOM] SOCKS5 proxy: {}", config.listen);
    info!("[PHANTOM] Padding: {}", if config.padding { "ON" } else { "OFF" });
    if let Some(ref edge) = config.cdn_edge {
        info!("[PHANTOM] CDN edge: {}", edge);
    }
    if let Some(ref host) = config.host_override {
        info!("[PHANTOM] Host override: {}", host);
    }
    if config.fragment {
        info!("[PHANTOM] TLS fragment: ON ({}B chunks)", config.fragment_size);
    }
    info!("[PHANTOM] ═══════════════════════════════════════");

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
        fragment_enabled: config.fragment,
        fragment_size: config.fragment_size,
        padding_enabled: config.padding,
    };

    // Spawn the transport loop (WebSocket, HTTP, or Auto)
    info!("[PHANTOM] Launching transport...");
    let transport_state = tunnel_state.clone();
    tokio::spawn(async move {
        transport::run_transport(transport_config, upstream_rx, transport_state).await;
    });

    // Open SOCKS5 listener
    let listener = match TcpListener::bind(&config.listen).await {
        Ok(l) => l,
        Err(e) => {
            error!("[PHANTOM] ❌ Failed to bind SOCKS5 on {}: {} (port in use?)", config.listen, e);
            return;
        }
    };

    info!("[PHANTOM] ✓ SOCKS5 proxy listening on {}", config.listen);

    let next_stream_id = Arc::new(std::sync::atomic::AtomicU32::new(1));

    loop {
        let (socket, peer) = listener.accept().await.unwrap();
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
            frames.push(Frame::connect(stream_id, &addr));
            // Store the reply sender for when we get CONNECT_OK/ERR back
            tunnel_state
                .lock()
                .await
                .connect_replies
                .insert(stream_id, reply);
        }
        UpstreamMsg::Data { stream_id, data } => {
            // Split large data into multiple frames (max 65535 bytes per frame)
            for chunk in data.chunks(32768) {
                frames.push(Frame::data(stream_id, chunk.to_vec()));
            }
        }
        UpstreamMsg::Close { stream_id } => {
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
                if let Some(reply) = state.connect_replies.remove(&frame.stream_id) {
                    let _ = reply.send(true);
                }
            }
            Cmd::ConnectErr => {
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
                if let Some(tx) = state.downstream_txs.get(&frame.stream_id) {
                    if tx.send(frame.data).await.is_err() {
                        state.downstream_txs.remove(&frame.stream_id);
                    }
                }
            }
            Cmd::Close => {
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
    if n < 4 || buf[0] != 0x05 || buf[1] != 0x01 {
        return Err("not a CONNECT request".into());
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
            let port =
                u16::from_be_bytes([buf[5 + dlen], buf[5 + dlen + 1]]);
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
        .map_err(|_| format!("connect timeout: {} did not respond in 15s (tunnel may be down)", addr))?
        .map_err(|_| format!("connect reply dropped for {} (transport reconnecting?)", addr))?;

    if !connected {
        // Send SOCKS5 failure response
        error!("[PHANTOM] Stream {} remote connect to {} FAILED", stream_id, addr);
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
