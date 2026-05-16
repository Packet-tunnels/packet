// phantom-server: Covert HTTP tunnel server with WebSocket + Relay support
//
// This server does four things simultaneously:
// 1. Serves a real static website (piano lessons) to look legitimate
// 2. Provides authenticated HTTP POST tunnel API for Phantom clients
// 3. Provides authenticated WebSocket tunnel for persistent connections
// 4. Accepts Starlink relay nodes that provide unfiltered internet exit
//
// Relay Architecture (Starlink bypass):
//   Mobile Client (Iran) → GCP Server → Relay Node (Starlink) → Free Internet
//   The relay node connects OUTBOUND to GCP, so it needs no public IP.
//   GCP forwards all client traffic through the relay for internet access.

use axum::{
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    extract::State,
    http::StatusCode,
    response::{Html, IntoResponse},
    routing::{get, post},
    Json, Router,
};
use clap::Parser;
use futures_util::{SinkExt, StreamExt};
use phantom_proto::*;
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::OwnedWriteHalf;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, Mutex};
use tracing::{error, info, warn};

// ─── CLI ───────────────────────────────────────────────────────
#[derive(Parser)]
#[command(name = "phantom-server")]
#[command(about = "Phantom Tunnel Server — covert HTTP tunnel with WebSocket + Relay")]
struct Cli {
    /// Port to listen on
    #[arg(short, long, default_value = "80")]
    port: u16,

    /// Shared secret for authentication
    #[arg(short, long, env = "PHANTOM_SECRET")]
    secret: String,

    /// Max allowed auth timestamp drift in seconds
    #[arg(long, default_value = "120")]
    max_drift: u64,
}

// ─── Server State ──────────────────────────────────────────────

struct AppState {
    secret: String,
    key: [u8; 32],
    max_drift: u64,
    sessions: Mutex<HashMap<String, Arc<Session>>>,
    recent_auth_nonces: Mutex<HashMap<String, u64>>,
    /// Active relay nodes (Starlink exit nodes)
    relay_nodes: Mutex<Vec<Arc<RelayNode>>>,
}

impl AppState {
    /// Get the best available relay node, or None to use direct exit.
    async fn get_relay(&self) -> Option<Arc<RelayNode>> {
        let relays = self.relay_nodes.lock().await;
        // Pick the relay with fewest active streams (simple load balancing)
        relays
            .iter()
            .filter(|r| r.is_alive())
            .min_by_key(|r| r.active_streams.load(std::sync::atomic::Ordering::Relaxed))
            .cloned()
    }

    async fn register_auth_nonce(&self, nonce: &str) -> bool {
        if nonce.is_empty() {
            return false;
        }

        let now = unix_now_secs();
        let ttl = self.max_drift.saturating_add(30);
        let mut nonces = self.recent_auth_nonces.lock().await;
        nonces.retain(|_, seen_at| now.saturating_sub(*seen_at) <= ttl);

        if nonces.contains_key(nonce) {
            return false;
        }

        nonces.insert(nonce.to_string(), now);
        true
    }
}

struct Session {
    key: [u8; 32],
    writers: Mutex<HashMap<u32, OwnedWriteHalf>>,
    downstream_tx: mpsc::Sender<Frame>,
    downstream_rx: Mutex<mpsc::Receiver<Frame>>,
    last_seen_unix: AtomicU64,
}

impl Session {
    fn new(key: [u8; 32]) -> Self {
        let (tx, rx) = mpsc::channel(4096);
        Self {
            key,
            writers: Mutex::new(HashMap::new()),
            downstream_tx: tx,
            downstream_rx: Mutex::new(rx),
            last_seen_unix: AtomicU64::new(unix_now_secs()),
        }
    }

    fn touch(&self) {
        self.last_seen_unix
            .store(unix_now_secs(), Ordering::Relaxed);
    }

    fn is_recently_active(&self, now: u64, max_idle_secs: u64) -> bool {
        now.saturating_sub(self.last_seen_unix.load(Ordering::Relaxed)) < max_idle_secs
    }
}

const SESSION_IDLE_TTL_SECS: u64 = 15 * 60;

struct ValidatedTransportAuth {
    key: [u8; 32],
    mode: &'static str,
    subject: String,
    bridge_id: Option<String>,
}

fn unix_now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

// ─── Relay Node ────────────────────────────────────────────────
// A relay node is a Starlink terminal (or any unfiltered exit) that
// maintains a persistent WebSocket connection to this server.
// When clients need to connect to the internet, the server forwards
// Connect/Data/Close frames to the relay, which makes the actual
// TCP connections and returns data.

struct RelayNode {
    /// Channel to send frames TO the relay node
    tx: mpsc::Sender<Frame>,
    /// Track pending connect responses from relay
    pending_connects: Mutex<HashMap<u32, mpsc::Sender<Frame>>>,
    /// Number of active streams through this relay
    active_streams: std::sync::atomic::AtomicU32,
    /// Whether the relay is still connected
    alive: std::sync::atomic::AtomicBool,
    /// Label for logging
    label: String,
}

impl RelayNode {
    fn new(tx: mpsc::Sender<Frame>, label: String) -> Self {
        Self {
            tx,
            pending_connects: Mutex::new(HashMap::new()),
            active_streams: std::sync::atomic::AtomicU32::new(0),
            alive: std::sync::atomic::AtomicBool::new(true),
            label,
        }
    }

    fn is_alive(&self) -> bool {
        self.alive.load(std::sync::atomic::Ordering::Relaxed)
    }

    fn mark_dead(&self) {
        self.alive
            .store(false, std::sync::atomic::Ordering::Relaxed);
    }
}

// ─── Main ──────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_target(false)
        .with_thread_ids(true)
        .init();

    let cli = Cli::parse();
    let key = derive_key(&cli.secret);

    info!("[PHANTOM] Server v0.3.0 starting on port {}", cli.port);
    info!("[PHANTOM] Max auth drift: {}s", cli.max_drift);
    info!("[PHANTOM] Relay node support: ENABLED");

    let state = Arc::new(AppState {
        secret: cli.secret.clone(),
        key,
        max_drift: cli.max_drift,
        sessions: Mutex::new(HashMap::new()),
        recent_auth_nonces: Mutex::new(HashMap::new()),
        relay_nodes: Mutex::new(Vec::new()),
    });

    // Spawn session cleanup task
    let cleanup_state = state.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(300)).await;
            let mut sessions = cleanup_state.sessions.lock().await;
            let now = unix_now_secs();
            // WebSocket sessions keep extra Arc references while open. HTTP polling
            // sessions do not, so retain recently active sessions by idle TTL too.
            sessions.retain(|_token, session| {
                Arc::strong_count(session) > 1
                    || session.is_recently_active(now, SESSION_IDLE_TTL_SECS)
            });
            // Clean dead relay nodes
            let mut relays = cleanup_state.relay_nodes.lock().await;
            relays.retain(|r| r.is_alive());
            let mut auth_nonces = cleanup_state.recent_auth_nonces.lock().await;
            let nonce_ttl = cleanup_state.max_drift.saturating_add(30);
            auth_nonces.retain(|_, seen_at| now.saturating_sub(*seen_at) <= nonce_ttl);
            info!(
                "Cleanup: {} active sessions, {} relay nodes, {} auth nonces",
                sessions.len(),
                relays.len(),
                auth_nonces.len(),
            );
        }
    });

    let app = Router::new()
        // Real website — serves to probes and censors
        .route("/", get(serve_homepage))
        .route("/about", get(serve_about))
        .route("/contact", get(serve_contact))
        // Tunnel API — looks like a normal web app API
        .route("/api/v1/auth/login", post(handle_auth))
        .route("/api/v1/lessons/sync", post(handle_sync))
        // WebSocket endpoint — looks like a live lesson streaming feature
        .route("/api/v1/lessons/live", get(ws_upgrade))
        // Relay endpoint — looks like a teacher's streaming setup
        .route("/api/v1/lessons/broadcast", get(relay_upgrade))
        // Health check (looks normal)
        .route("/api/v1/health", get(health_check))
        .with_state(state);

    let addr = format!("0.0.0.0:{}", cli.port);
    let listener = TcpListener::bind(&addr).await.unwrap();
    info!("Listening on {}", addr);
    axum::serve(listener, app).await.unwrap();
}

// ─── Real Website Handlers (Camouflage) ────────────────────────

async fn serve_homepage() -> Html<&'static str> {
    Html(include_str!("../../static/index.html"))
}

async fn serve_about() -> Html<&'static str> {
    Html(
        r#"<!DOCTYPE html>
<html lang="fa" dir="rtl"><head><meta charset="utf-8">
<link href="https://cdn.jsdelivr.net/gh/rastikerdar/vazirmatn@v33.0.0/Vazirmatn-font-face.css" rel="stylesheet" type="text/css" />
<style>body{background:#0f1014;color:#e2e4e9;font-family:'Vazirmatn',sans-serif;text-align:center;padding:100px;line-height:1.8;} a{color:#d4af37;text-decoration:none;}</style>
<title>درباره ما - استودیو آرتین</title></head>
<body><h1>درباره استودیو آرتین</h1>
<p>ما گروهی از معماران و طراحان خلاق هستیم که به زیبایی در سادگی باور داریم.</p>
<p><a href="/">← بازگشت به خانه</a></p></body></html>"#,
    )
}

async fn serve_contact() -> Html<&'static str> {
    Html(
        r#"<!DOCTYPE html>
<html lang="fa" dir="rtl"><head><meta charset="utf-8">
<link href="https://cdn.jsdelivr.net/gh/rastikerdar/vazirmatn@v33.0.0/Vazirmatn-font-face.css" rel="stylesheet" type="text/css" />
<style>body{background:#0f1014;color:#e2e4e9;font-family:'Vazirmatn',sans-serif;text-align:center;padding:100px;line-height:1.8;} a{color:#d4af37;text-decoration:none;}</style>
<title>تماس با ما - استودیو آرتین</title></head>
<body><h1>ارتباط با ما</h1>
<p>برای رزرو مشاوره طراحی با ایمیل info@artin-studio.com در ارتباط باشید.</p>
<p><a href="/">← بازگشت به خانه</a></p></body></html>"#,
    )
}

async fn health_check(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let relay_count = state
        .relay_nodes
        .lock()
        .await
        .iter()
        .filter(|r| r.is_alive())
        .count();
    Json(serde_json::json!({
        "status": "ok",
        "service": "piano-lessons-api",
        "features": relay_count,
    }))
}

// ─── Authentication ────────────────────────────────────────────

async fn handle_auth(
    State(state): State<Arc<AppState>>,
    Json(req): Json<AuthRequest>,
) -> impl IntoResponse {
    let auth = match validate_transport_auth(&state, &req).await {
        Ok(auth) => auth,
        Err(error) => {
            warn!("Auth rejected: {}", error);
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({"error": "invalid credentials"})),
            );
        }
    };

    let token = generate_session_token();
    let session = Arc::new(Session::new(auth.key));

    state.sessions.lock().await.insert(token.clone(), session);

    info!(
        "New session created: {}... mode={} subject={}",
        &token[..16],
        auth.mode,
        auth.subject
    );

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "token": token,
            "auth_mode": auth.mode,
            "bridge_id": auth.bridge_id,
        })),
    )
}

async fn validate_transport_auth(
    state: &Arc<AppState>,
    auth: &AuthRequest,
) -> Result<ValidatedTransportAuth, String> {
    if let Some(ticket) = auth.ticket.as_deref().filter(|ticket| !ticket.trim().is_empty()) {
        let claims = verify_transport_ticket(&state.secret, ticket, unix_now_secs())
            .map_err(|error| error.to_string())?;
        let nonce_key = format!("ticket:{}", claims.jti);
        if !state.register_auth_nonce(&nonce_key).await {
            return Err("replayed transport ticket".to_string());
        }
        let key = claims
            .session_key_bytes()
            .map_err(|error| error.to_string())?;
        return Ok(ValidatedTransportAuth {
            key,
            mode: "ticket",
            subject: claims.sub,
            bridge_id: claims.bridge_id,
        });
    }

    let now = unix_now_secs();
    let drift = if auth.ts > now {
        auth.ts - now
    } else {
        now - auth.ts
    };
    if drift > state.max_drift {
        return Err(format!("timestamp drift {}s", drift));
    }

    if !verify_auth(&state.secret, auth.ts, &auth.n, &auth.sig) {
        return Err("bad legacy signature".to_string());
    }

    if !state.register_auth_nonce(&auth.n).await {
        return Err("replayed legacy nonce".to_string());
    }

    Ok(ValidatedTransportAuth {
        key: state.key,
        mode: "legacy",
        subject: "shared-secret".to_string(),
        bridge_id: None,
    })
}

async fn validate_relay_auth(
    state: &Arc<AppState>,
    auth_json: serde_json::Value,
) -> Result<(ValidatedTransportAuth, String), String> {
    let auth: AuthRequest =
        serde_json::from_value(auth_json.clone()).map_err(|_| "invalid relay request".to_string())?;
    if auth.mode.as_deref() != Some("relay") {
        return Err("invalid relay mode".to_string());
    }

    let relay_label = auth_json["label"].as_str().unwrap_or("unknown").to_string();
    let validated = validate_transport_auth(state, &auth).await?;
    if validated.mode == "ticket" {
        let claims = auth
            .ticket
            .as_deref()
            .and_then(|ticket| decode_transport_ticket(ticket).ok())
            .ok_or_else(|| "invalid relay ticket".to_string())?;
        if !claims.capabilities.iter().any(|capability| capability == "relay") {
            return Err("relay ticket missing capability".to_string());
        }
    }

    Ok((validated, relay_label))
}

// ═══════════════════════════════════════════════════════════════
// Relay Node Handler (Starlink Exit Node)
// ═══════════════════════════════════════════════════════════════
//
// A relay node (running on Starlink) connects here and says:
// "I can make TCP connections to the open internet."
//
// The server then routes client traffic through this relay
// instead of making direct connections from GCP.
//
// To censors, this endpoint looks like a "teacher broadcasting"
// their lesson content — a normal WebSocket for video streaming.

async fn relay_upgrade(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_relay_websocket(socket, state))
}

async fn handle_relay_websocket(socket: WebSocket, state: Arc<AppState>) {
    let (mut ws_sender, mut ws_receiver) = socket.split();
    info!("[RELAY] New relay connection, waiting for auth...");

    // ── Step 1: Authenticate ──
    let auth_msg = match tokio::time::timeout(Duration::from_secs(10), ws_receiver.next()).await {
        Ok(Some(Ok(Message::Text(text)))) => text,
        _ => {
            warn!("[RELAY] Auth failed: no valid message received");
            return;
        }
    };

    let auth_json: serde_json::Value = match serde_json::from_str(&auth_msg) {
        Ok(v) => v,
        Err(_) => {
            let _ = ws_sender
                .send(Message::Text(r#"{"error":"invalid request"}"#.into()))
                .await;
            return;
        }
    };

    let (relay_auth, relay_label) = match validate_relay_auth(&state, auth_json).await {
        Ok(result) => result,
        Err(error) => {
            warn!("[RELAY] Auth rejected: {}", error);
            let _ = ws_sender
                .send(Message::Text(r#"{"error":"unauthorized"}"#.into()))
                .await;
            return;
        }
    };

    // Accept the relay
    let relay_id = generate_session_token();
    let _ = ws_sender
        .send(Message::Text(
            serde_json::json!({"relay_id": &relay_id, "status": "accepted"}).to_string(),
        ))
        .await;

    info!(
        "[RELAY] ✓ Relay node '{}' authenticated: {}...",
        relay_label,
        &relay_id[..16]
    );

    let key = relay_auth.key;

    // Channel for sending frames TO the relay (server → relay)
    let (relay_tx, mut relay_rx) = mpsc::channel::<Frame>(4096);

    let relay_node = Arc::new(RelayNode::new(relay_tx, relay_label.clone()));

    // Register the relay
    {
        let mut relays = state.relay_nodes.lock().await;
        relays.push(relay_node.clone());
        info!("[RELAY] {} relay nodes now active", relays.len());
    }

    // ── Sender task: frames from server → encrypt → WS → relay node ──
    let sender_relay = relay_node.clone();
    let sender_task = tokio::spawn(async move {
        loop {
            let mut frames = Vec::new();

            tokio::select! {
                result = relay_rx.recv() => {
                    match result {
                        Some(frame) => frames.push(frame),
                        None => break,
                    }
                }
                _ = tokio::time::sleep(Duration::from_secs(25)) => {
                    // Keepalive ping
                    if ws_sender.send(Message::Ping(vec![0x52, 0x4C])).await.is_err() {
                        break;
                    }
                    continue;
                }
            }

            // Drain queued frames
            while let Ok(f) = relay_rx.try_recv() {
                frames.push(f);
                if frames.len() >= 256 {
                    break;
                }
            }

            if !frames.is_empty() {
                let plaintext = encode_frames(&frames);
                let encrypted = encrypt(&key, &plaintext);
                if ws_sender.send(Message::Binary(encrypted)).await.is_err() {
                    break;
                }
            }
        }
        sender_relay.mark_dead();
    });

    // ── Receiver task: WS → decrypt → frames from relay node → route back to client sessions ──
    let receiver_relay = relay_node.clone();
    let receiver_state = state.clone();
    let receiver_task = tokio::spawn(async move {
        while let Some(msg) = ws_receiver.next().await {
            match msg {
                Ok(Message::Binary(data)) => {
                    match decrypt(&key, &data) {
                        Ok(plaintext) => {
                            match decode_frames(&plaintext) {
                                Ok(frames) => {
                                    for frame in frames {
                                        // Route relay responses back to the appropriate client session
                                        relay_dispatch_to_client(
                                            frame,
                                            &receiver_relay,
                                            &receiver_state,
                                        )
                                        .await;
                                    }
                                }
                                Err(e) => error!("[RELAY] Frame decode error: {}", e),
                            }
                        }
                        Err(e) => error!("[RELAY] Decrypt error: {}", e),
                    }
                }
                Ok(Message::Close(_)) => {
                    info!("[RELAY] Relay node disconnected cleanly");
                    break;
                }
                Err(e) => {
                    warn!("[RELAY] WS error: {}", e);
                    break;
                }
                _ => {}
            }
        }
        receiver_relay.mark_dead();
    });

    tokio::select! {
        _ = sender_task => {},
        _ = receiver_task => {},
    }

    // Remove dead relay
    relay_node.mark_dead();
    let mut relays = state.relay_nodes.lock().await;
    relays.retain(|r| r.is_alive());
    info!(
        "[RELAY] Relay '{}' removed. {} nodes remaining.",
        relay_label,
        relays.len()
    );
}

/// Route a frame received FROM the relay back to the client session that requested it.
/// The relay sends ConnectOk/ConnectErr/Data/Close frames with stream_ids that map
/// to the original client session's stream_ids.
async fn relay_dispatch_to_client(frame: Frame, relay: &Arc<RelayNode>, _state: &Arc<AppState>) {
    let stream_id = frame.stream_id;
    let pending = relay.pending_connects.lock().await;
    if let Some(session_tx) = pending.get(&stream_id) {
        if session_tx.send(frame).await.is_err() {
            // Session is dead, clean up
            drop(pending);
            relay.pending_connects.lock().await.remove(&stream_id);
            relay
                .active_streams
                .fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
        }
    }
}

// ─── WebSocket Tunnel ──────────────────────────────────────────

async fn ws_upgrade(ws: WebSocketUpgrade, State(state): State<Arc<AppState>>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_websocket(socket, state))
}

async fn handle_websocket(socket: WebSocket, state: Arc<AppState>) {
    let (mut ws_sender, mut ws_receiver) = socket.split();
    info!("[PHANTOM] WS: new connection, waiting for auth...");

    // ── Step 1: Authenticate via first message ──
    let auth_msg = match tokio::time::timeout(Duration::from_secs(10), ws_receiver.next()).await {
        Ok(Some(Ok(Message::Text(text)))) => text,
        Ok(Some(Ok(other))) => {
            warn!("[PHANTOM] WS auth: expected Text, got {:?}", other);
            return;
        }
        Ok(Some(Err(e))) => {
            warn!("[PHANTOM] WS auth: receive error: {}", e);
            return;
        }
        Ok(None) => {
            warn!("[PHANTOM] WS auth: connection closed before auth");
            return;
        }
        Err(_) => {
            warn!("[PHANTOM] WS auth: 10s timeout — client never sent auth");
            return;
        }
    };

    let auth: AuthRequest = match serde_json::from_str(&auth_msg) {
        Ok(a) => a,
        Err(e) => {
            warn!(
                "[PHANTOM] WS auth: invalid JSON: {} — raw: '{}'",
                e,
                &auth_msg[..auth_msg.len().min(100)]
            );
            let _ = ws_sender
                .send(Message::Text(r#"{"error":"invalid request"}"#.into()))
                .await;
            return;
        }
    };

    let validated = match validate_transport_auth(&state, &auth).await {
        Ok(validated) => validated,
        Err(error) => {
            warn!("[PHANTOM] WS auth rejected: {}", error);
            let _ = ws_sender
                .send(Message::Text(r#"{"error":"unauthorized"}"#.into()))
                .await;
            return;
        }
    };

    let token = generate_session_token();
    let session = Arc::new(Session::new(validated.key));
    state
        .sessions
        .lock()
        .await
        .insert(token.clone(), session.clone());

    let has_relay = state.get_relay().await.is_some();

    let _ = ws_sender
        .send(Message::Text(
            serde_json::json!({
                "token": &token,
                "relay": has_relay,
                "auth_mode": validated.mode,
                "bridge_id": validated.bridge_id,
            }).to_string(),
        ))
        .await;

    info!(
        "[PHANTOM] ✓ WS session established: {}... (relay: {}, mode: {})",
        &token[..16],
        has_relay,
        validated.mode
    );

    let key = session.key;

    // ── Step 2: Bidirectional relay ──

    // Sender task: downstream frames → WebSocket
    let session_tx = session.clone();
    let sender_task = tokio::spawn(async move {
        let mut rx = session_tx.downstream_rx.lock().await;
        loop {
            let mut frames = Vec::new();

            tokio::select! {
                result = rx.recv() => {
                    match result {
                        Some(frame) => frames.push(frame),
                        None => break,
                    }
                }
                _ = tokio::time::sleep(Duration::from_secs(25)) => {
                    if ws_sender.send(Message::Ping(vec![0x50, 0x48])).await.is_err() {
                        break;
                    }
                    continue;
                }
            }

            while let Ok(f) = rx.try_recv() {
                frames.push(f);
                if frames.len() >= 256 {
                    break;
                }
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

    // Receiver task: WebSocket → upstream frame processing
    let session_rx = session.clone();
    let rx_state = state.clone();
    let receiver_task = tokio::spawn(async move {
        while let Some(msg) = ws_receiver.next().await {
            match msg {
                Ok(Message::Binary(data)) => match decrypt(&key, &data) {
                    Ok(plaintext) => match decode_frames(&plaintext) {
                        Ok(frames) => {
                            for frame in frames {
                                process_upstream_frame_with_relay(frame, &session_rx, &rx_state)
                                    .await;
                            }
                        }
                        Err(e) => {
                            error!("[PHANTOM] Frame decode error: {} ({}B)", e, plaintext.len())
                        }
                    },
                    Err(e) => error!(
                        "[PHANTOM] Decrypt error from client: {} ({}B — key mismatch?)",
                        e,
                        data.len()
                    ),
                },
                Ok(Message::Ping(_)) => {}
                Ok(Message::Close(reason)) => {
                    info!("[PHANTOM] WS closed by client: {:?}", reason);
                    break;
                }
                Err(e) => {
                    warn!("[PHANTOM] WS receive error: {}", e);
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

    state.sessions.lock().await.remove(&token);
    info!("[PHANTOM] WS session closed: {}...", &token[..16]);
}

// ─── Frame Processing (with Relay Support) ─────────────────────
// If a relay node is available, forward Connect/Data/Close to the relay.
// Otherwise, make direct TCP connections from the server (fallback).

async fn process_upstream_frame_with_relay(
    frame: Frame,
    session: &Arc<Session>,
    state: &Arc<AppState>,
) {
    // Try to get a relay node
    let relay = state.get_relay().await;

    match frame.cmd {
        Cmd::Connect => {
            let addr = String::from_utf8_lossy(&frame.data).to_string();
            let stream_id = frame.stream_id;

            if let Err(reason) = validate_outbound_target(&addr).await {
                warn!(
                    "[PHANTOM] Stream {} blocked outbound target {}: {}",
                    stream_id, addr, reason
                );
                let _ = session
                    .downstream_tx
                    .send(Frame::connect_err(
                        stream_id,
                        "destination blocked by server policy",
                    ))
                    .await;
                return;
            }

            if let Some(relay) = relay {
                // ── Route through relay (Starlink exit) ──
                info!(
                    "[RELAY-ROUTE] Stream {} → relay '{}' → {}",
                    stream_id, relay.label, addr
                );

                // Register this stream's session downstream_tx so relay responses come back
                relay
                    .pending_connects
                    .lock()
                    .await
                    .insert(stream_id, session.downstream_tx.clone());
                relay
                    .active_streams
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

                // Forward the Connect frame to the relay
                if relay
                    .tx
                    .send(Frame::connect(stream_id, &addr))
                    .await
                    .is_err()
                {
                    error!("[RELAY-ROUTE] Failed to send to relay, falling back to direct");
                    relay.mark_dead();
                    // Fallback: direct connection
                    process_upstream_frame_direct(frame, session).await;
                }
            } else {
                // ── Direct connection (no relay available) ──
                process_upstream_frame_direct(frame, session).await;
            }
        }

        Cmd::Data => {
            if let Some(relay) = relay {
                // Check if this stream is routed through a relay
                let pending = relay.pending_connects.lock().await;
                if pending.contains_key(&frame.stream_id) {
                    drop(pending);
                    // Forward data to relay
                    if relay.tx.send(frame).await.is_err() {
                        relay.mark_dead();
                    }
                    return;
                }
            }
            // Direct path: write to local TCP connection
            let mut writers = session.writers.lock().await;
            if let Some(writer) = writers.get_mut(&frame.stream_id) {
                if let Err(e) = writer.write_all(&frame.data).await {
                    error!("Stream {} write error: {}", frame.stream_id, e);
                    writers.remove(&frame.stream_id);
                    let _ = session
                        .downstream_tx
                        .send(Frame::close(frame.stream_id))
                        .await;
                }
            }
        }

        Cmd::Close => {
            info!("Stream {} closed by client", frame.stream_id);
            if let Some(relay) = relay {
                let mut pending = relay.pending_connects.lock().await;
                if pending.remove(&frame.stream_id).is_some() {
                    relay
                        .active_streams
                        .fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
                    drop(pending);
                    let _ = relay.tx.send(frame).await;
                    return;
                }
            }
            session.writers.lock().await.remove(&frame.stream_id);
        }

        _ => {}
    }
}

/// Direct connection (fallback when no relay is available)
async fn process_upstream_frame_direct(frame: Frame, session: &Arc<Session>) {
    match frame.cmd {
        Cmd::Connect => {
            let addr = String::from_utf8_lossy(&frame.data).to_string();
            let stream_id = frame.stream_id;
            let tx = session.downstream_tx.clone();
            let session = session.clone();

            info!("[DIRECT] Stream {} connecting to {}", stream_id, addr);

            tokio::spawn(async move {
                match TcpStream::connect(&addr).await {
                    Ok(tcp) => {
                        let (mut read_half, write_half) = tcp.into_split();
                        session.writers.lock().await.insert(stream_id, write_half);

                        let _ = tx.send(Frame::connect_ok(stream_id)).await;
                        info!("[DIRECT] Stream {} connected to {}", stream_id, addr);

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
                                        let _ = tx2
                                            .send(Frame::data(stream_id, buf[..n].to_vec()))
                                            .await;
                                    }
                                    Err(e) => {
                                        error!("Stream {} read error: {}", stream_id, e);
                                        let _ = tx2.send(Frame::close(stream_id)).await;
                                        break;
                                    }
                                }
                            }
                        });
                    }
                    Err(e) => {
                        error!("[DIRECT] Stream {} connect failed: {}", stream_id, e);
                        let _ = tx.send(Frame::connect_err(stream_id, &e.to_string())).await;
                    }
                }
            });
        }
        _ => {}
    }
}

// ─── HTTP Tunnel Sync ──────────────────────────────────────────
// Original HTTP POST-based tunnel (backward compatible).
// Still works for direct connections outside Iran.

async fn handle_sync(
    State(state): State<Arc<AppState>>,
    Json(req): Json<SyncRequest>,
) -> impl IntoResponse {
    // Look up session
    let session = {
        let sessions = state.sessions.lock().await;
        sessions.get(&req.t).cloned()
    };

    let session = match session {
        Some(s) => s,
        None => {
            return (
                StatusCode::OK,
                Json(serde_json::json!({"error": "session expired"})),
            );
        }
    };
    session.touch();

    // Decrypt and decode upstream frames
    let encrypted = match b64_decode(&req.d) {
        Ok(d) => d,
        Err(_) => {
            return (StatusCode::OK, Json(serde_json::json!({"d": ""})));
        }
    };

    let plaintext = match decrypt(&session.key, &encrypted) {
        Ok(p) => p,
        Err(_) => {
            return (StatusCode::OK, Json(serde_json::json!({"d": ""})));
        }
    };

    let upstream_frames = decode_frames(&plaintext).unwrap_or_default();

    // Process upstream frames (with relay support)
    for frame in upstream_frames {
        process_upstream_frame_with_relay(frame, &session, &state).await;
    }

    // Give spawned tasks a moment to produce downstream data
    tokio::time::sleep(Duration::from_millis(20)).await;

    // Collect downstream frames
    let mut downstream_frames = Vec::new();
    {
        let mut rx = session.downstream_rx.lock().await;
        while let Ok(frame) = rx.try_recv() {
            downstream_frames.push(frame);
            if downstream_frames.len() >= 256 {
                break;
            }
        }
    }

    // Encode, encrypt, base64
    let plaintext = encode_frames(&downstream_frames);
    let encrypted = encrypt(&session.key, &plaintext);
    let encoded = b64_encode(&encrypted);

    (StatusCode::OK, Json(serde_json::json!({"d": encoded})))
}

async fn validate_outbound_target(addr: &str) -> Result<(), String> {
    let (host, port) = split_target_host_port(addr)?;

    if let Ok(ip) = host.parse::<IpAddr>() {
        return validate_public_ip(ip);
    }

    let resolved = tokio::time::timeout(
        Duration::from_secs(5),
        tokio::net::lookup_host((host.as_str(), port)),
    )
    .await
    .map_err(|_| format!("DNS lookup timed out for {}", host))?
    .map_err(|e| format!("DNS lookup failed for {}: {}", host, e))?;

    let mut saw_address = false;
    for socket_addr in resolved {
        saw_address = true;
        validate_public_ip(socket_addr.ip())?;
    }

    if !saw_address {
        return Err(format!("{} resolved to no usable addresses", host));
    }

    Ok(())
}

fn split_target_host_port(addr: &str) -> Result<(String, u16), String> {
    if let Ok(socket_addr) = addr.parse::<SocketAddr>() {
        return Ok((socket_addr.ip().to_string(), socket_addr.port()));
    }

    let (host, port) = addr
        .rsplit_once(':')
        .ok_or_else(|| format!("missing port in {}", addr))?;
    let host = host.trim().trim_start_matches('[').trim_end_matches(']');
    let port = port
        .parse::<u16>()
        .map_err(|_| format!("invalid port in {}", addr))?;

    if host.is_empty() {
        return Err(format!("missing host in {}", addr));
    }

    Ok((host.to_string(), port))
}

fn validate_public_ip(ip: IpAddr) -> Result<(), String> {
    if let Some(reason) = blocked_ip_reason(ip) {
        return Err(format!("{} is {}", ip, reason));
    }

    Ok(())
}

fn blocked_ip_reason(ip: IpAddr) -> Option<&'static str> {
    match ip {
        IpAddr::V4(ipv4) => blocked_ipv4_reason(ipv4),
        IpAddr::V6(ipv6) => blocked_ipv6_reason(ipv6),
    }
}

fn blocked_ipv4_reason(ip: Ipv4Addr) -> Option<&'static str> {
    let octets = ip.octets();

    if ip.is_private() {
        Some("private")
    } else if ip.is_loopback() {
        Some("loopback")
    } else if ip.is_link_local() {
        Some("link-local")
    } else if ip.is_multicast() {
        Some("multicast")
    } else if ip.is_broadcast() {
        Some("broadcast")
    } else if ip.is_documentation() {
        Some("documentation-only")
    } else if ip.is_unspecified() {
        Some("unspecified")
    } else if octets[0] == 100 && (octets[1] & 0b1100_0000) == 64 {
        Some("carrier-grade NAT")
    } else if octets[0] == 198 && matches!(octets[1], 18 | 19) {
        Some("benchmarking-only")
    } else if octets[0] >= 240 {
        Some("reserved")
    } else {
        None
    }
}

fn blocked_ipv6_reason(ip: Ipv6Addr) -> Option<&'static str> {
    let segments = ip.segments();
    let first = segments[0];

    if ip.is_loopback() {
        Some("loopback")
    } else if ip.is_unspecified() {
        Some("unspecified")
    } else if ip.is_multicast() {
        Some("multicast")
    } else if (first & 0xfe00) == 0xfc00 {
        Some("unique-local")
    } else if (first & 0xffc0) == 0xfe80 {
        Some("link-local")
    } else if (first & 0xffc0) == 0xfec0 {
        Some("site-local")
    } else if first == 0x2001 && segments[1] == 0x0db8 {
        Some("documentation-only")
    } else {
        None
    }
}
