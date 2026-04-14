// phantom-client: Covert HTTP tunnel client
//
// This client does three things:
// 1. Opens a local SOCKS5 proxy (default :1080) for apps to connect through
// 2. Multiplexes all SOCKS5 streams into encrypted HTTP POST requests
// 3. Sends those requests to the Phantom server disguised as API calls
//
// Traffic pattern: rapid short-lived HTTP POST/response cycles
// that look like a web app making API calls — not a persistent tunnel.


pub mod ffi;

use phantom_proto::*;
use reqwest::Client;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::sync::Mutex;
use tracing::{error, info, warn};

// ─── Messages between SOCKS5 handlers and the poller ───────────

enum UpstreamMsg {
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

// ─── Shared Tunnel State ───────────────────────────────────────

struct TunnelState {
    /// Send downstream data to SOCKS5 handlers
    downstream_txs: HashMap<u32, mpsc::Sender<Vec<u8>>>,
    /// Pending connect replies
    connect_replies: HashMap<u32, tokio::sync::oneshot::Sender<bool>>,
}

// ─── Main ──────────────────────────────────────────────────────

pub async fn start_client(server_url: String, secret: String, listen: String) {
    let key = derive_key(&secret);
    let server_url = server_url.trim_end_matches('/').to_string();

    info!("Phantom Client starting");
    info!("Server: {}", server_url);
    info!("SOCKS5 proxy: {}", listen);

    // Build HTTP client with connection pooling
    let http_client = Client::builder()
        .pool_max_idle_per_host(4)
        .timeout(Duration::from_secs(30))
        .danger_accept_invalid_certs(true) // for ArvanCloud CDN flexibility
        .build()
        .expect("failed to build HTTP client");

    // Authenticate with server
    let token = authenticate(&http_client, &server_url, &secret)
        .await
        .expect("authentication failed — check your secret and server URL");

    info!("Authenticated successfully. Session: {}...", &token[..16]);

    // Channels for SOCKS5 handler ↔ poller communication
    let (upstream_tx, upstream_rx) = mpsc::channel::<UpstreamMsg>(4096);

    let tunnel_state = Arc::new(Mutex::new(TunnelState {
        downstream_txs: HashMap::new(),
        connect_replies: HashMap::new(),
    }));

    // Spawn the central HTTP polling loop
    let poller_state = tunnel_state.clone();
    let poller_token = token.clone();
    let poller_key = key;
    let poller_url = server_url.clone();
    let poll_idle = Duration::from_millis(200);
    let poll_active = Duration::from_millis(50);

    tokio::spawn(async move {
        poller_loop(
            http_client,
            poller_url,
            poller_token,
            poller_key,
            upstream_rx,
            poller_state,
            poll_active,
            poll_idle,
        )
        .await;
    });

    // Open SOCKS5 listener
    let listener = TcpListener::bind(&listen)
        .await
        .expect("failed to bind SOCKS5 listener");

    info!("SOCKS5 proxy listening on {}", listen);

    let next_stream_id = Arc::new(std::sync::atomic::AtomicU32::new(1));

    loop {
        let (socket, peer) = listener.accept().await.unwrap();
        info!("SOCKS5 connection from {}", peer);

        let stream_id = next_stream_id.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let tx = upstream_tx.clone();
        let state = tunnel_state.clone();

        tokio::spawn(async move {
            if let Err(e) = handle_socks5(socket, stream_id, tx, state).await {
                warn!("SOCKS5 stream {} error: {}", stream_id, e);
            }
        });
    }
}

// ─── Authentication ────────────────────────────────────────────

async fn authenticate(
    client: &Client,
    server_url: &str,
    secret: &str,
) -> Result<String, String> {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let sig = sign_auth(secret, ts);

    let url = format!("{}/api/v1/auth/login", server_url);
    let body = AuthRequest { ts, sig };

    let resp = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("HTTP request failed: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("Auth failed with status {}", resp.status()));
    }

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Bad response: {}", e))?;

    json["token"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| "No token in response".to_string())
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
        .map_err(|_| "connect timeout")?
        .map_err(|_| "connect reply dropped")?;

    if !connected {
        // Send SOCKS5 failure response
        socket
            .write_all(&[0x05, 0x05, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
            .await?;
        return Err("remote connect failed".into());
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

// ─── Central HTTP Polling Loop ─────────────────────────────────
// This is the heart of the client. It collects upstream data from
// all SOCKS5 handlers, sends it in a single HTTP POST, and dispatches
// the response downstream to the appropriate handlers.

async fn poller_loop(
    client: Client,
    server_url: String,
    token: String,
    key: [u8; 32],
    mut upstream_rx: mpsc::Receiver<UpstreamMsg>,
    tunnel_state: Arc<Mutex<TunnelState>>,
    poll_active: Duration,
    poll_idle: Duration,
) {
    let sync_url = format!("{}/api/v1/lessons/sync", server_url);
    let mut last_data = Instant::now();
    let mut consecutive_empty: u32 = 0;

    loop {
        // Collect all pending upstream messages (non-blocking drain)
        let mut upstream_frames = Vec::new();

        // Try to receive at least one message (with timeout = poll interval)
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
                error!("Upstream channel closed, exiting poller");
                return;
            }
            Err(_) => {} // timeout, just poll
        }

        // Drain any remaining messages (non-blocking)
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

        match client.post(&sync_url).json(&req_body).send().await {
            Ok(resp) => {
                if let Ok(sync_resp) = resp.json::<SyncResponse>().await {
                    if let Ok(encrypted) = b64_decode(&sync_resp.d) {
                        if let Ok(plaintext) = decrypt(&key, &encrypted) {
                            if let Ok(frames) = decode_frames(&plaintext) {
                                if !frames.is_empty() {
                                    consecutive_empty = 0;
                                    last_data = Instant::now();
                                } else {
                                    consecutive_empty =
                                        consecutive_empty.saturating_add(1);
                                }

                                // Dispatch downstream frames
                                dispatch_downstream(frames, &tunnel_state).await;
                            }
                        }
                    }
                }
            }
            Err(e) => {
                warn!("Sync request failed: {}", e);
                // Back off on error
                tokio::time::sleep(Duration::from_secs(2)).await;
                consecutive_empty = 0;
            }
        }
    }
}

async fn process_upstream_msg(
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

async fn dispatch_downstream(
    frames: Vec<Frame>,
    tunnel_state: &Arc<Mutex<TunnelState>>,
) {
    let mut state = tunnel_state.lock().await;

    for frame in frames {
        match frame.cmd {
            Cmd::ConnectOk => {
                if let Some(reply) = state.connect_replies.remove(&frame.stream_id)
                {
                    let _ = reply.send(true);
                }
            }
            Cmd::ConnectErr => {
                warn!(
                    "Stream {} connect error: {}",
                    frame.stream_id,
                    String::from_utf8_lossy(&frame.data)
                );
                if let Some(reply) = state.connect_replies.remove(&frame.stream_id)
                {
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
