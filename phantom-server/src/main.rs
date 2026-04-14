// phantom-server: Covert HTTP tunnel server with WebSocket support
//
// This server does three things simultaneously:
// 1. Serves a real static website (piano lessons) to look legitimate
// 2. Provides authenticated HTTP POST tunnel API for Phantom clients
// 3. Provides authenticated WebSocket tunnel for persistent connections
//
// The tunnel data is hidden inside normal-looking JSON API calls
// and WebSocket frames. To censors and probes, this looks like
// a standard web application with real-time features.

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
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::net::tcp::OwnedWriteHalf;
use tokio::sync::{mpsc, Mutex};
use tracing::{error, info, warn};

// ─── CLI ───────────────────────────────────────────────────────
#[derive(Parser)]
#[command(name = "phantom-server")]
#[command(about = "Phantom Tunnel Server — covert HTTP tunnel with WebSocket")]
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
}

struct Session {
    writers: Mutex<HashMap<u32, OwnedWriteHalf>>,
    downstream_tx: mpsc::Sender<Frame>,
    downstream_rx: Mutex<mpsc::Receiver<Frame>>,
}

impl Session {
    fn new() -> Self {
        let (tx, rx) = mpsc::channel(4096);
        Self {
            writers: Mutex::new(HashMap::new()),
            downstream_tx: tx,
            downstream_rx: Mutex::new(rx),
        }
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

    info!("[PHANTOM] Server v0.2.0 starting on port {}", cli.port);
    info!("[PHANTOM] Max auth drift: {}s", cli.max_drift);

    let state = Arc::new(AppState {
        secret: cli.secret.clone(),
        key,
        max_drift: cli.max_drift,
        sessions: Mutex::new(HashMap::new()),
    });

    // Spawn session cleanup task
    let cleanup_state = state.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(300)).await;
            let mut sessions = cleanup_state.sessions.lock().await;
            // Remove sessions with no active streams and no references
            sessions.retain(|_token, session| Arc::strong_count(session) > 1);
            info!("Session cleanup: {} active sessions", sessions.len());
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
    Html(r#"<!DOCTYPE html>
<html lang="en"><head><meta charset="utf-8"><title>About - Piano Lessons Online</title>
<style>body{font-family:Georgia,serif;max-width:800px;margin:40px auto;padding:0 20px;color:#333;line-height:1.8}
h1{color:#2c3e50}a{color:#3498db}</style></head>
<body><h1>About Our Piano Lessons</h1>
<p>We offer comprehensive online piano lessons for all skill levels. Our certified instructors
bring decades of experience to help you master the piano from the comfort of your home.</p>
<p>Founded in 2024, we have helped over 500 students achieve their musical goals.</p>
<p><a href="/">← Back to Home</a></p></body></html>"#)
}

async fn serve_contact() -> Html<&'static str> {
    Html(r#"<!DOCTYPE html>
<html lang="en"><head><meta charset="utf-8"><title>Contact - Piano Lessons Online</title>
<style>body{font-family:Georgia,serif;max-width:800px;margin:40px auto;padding:0 20px;color:#333;line-height:1.8}
h1{color:#2c3e50}a{color:#3498db}</style></head>
<body><h1>Contact Us</h1>
<p>Email: info@piano-lessons.site</p>
<p>We typically respond within 24 hours.</p>
<p><a href="/">← Back to Home</a></p></body></html>"#)
}

async fn health_check() -> impl IntoResponse {
    Json(serde_json::json!({"status": "ok", "service": "piano-lessons-api"}))
}

// ─── Authentication ────────────────────────────────────────────

async fn handle_auth(
    State(state): State<Arc<AppState>>,
    Json(req): Json<AuthRequest>,
) -> impl IntoResponse {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    // Check timestamp drift
    let drift = if req.ts > now { req.ts - now } else { now - req.ts };
    if drift > state.max_drift {
        warn!("Auth rejected: timestamp drift {}s", drift);
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error": "invalid credentials"})),
        );
    }

    // Verify HMAC signature
    if !verify_auth(&state.secret, req.ts, &req.sig) {
        warn!("Auth rejected: bad signature");
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error": "invalid credentials"})),
        );
    }

    // Create session
    let token = generate_session_token();
    let session = Arc::new(Session::new());

    state
        .sessions
        .lock()
        .await
        .insert(token.clone(), session);

    info!("New session created: {}...", &token[..16]);

    (StatusCode::OK, Json(serde_json::json!({"token": token})))
}

// ─── WebSocket Tunnel ──────────────────────────────────────────
// Persistent bidirectional tunnel over WebSocket.
// Looks like a real-time lesson streaming feature to censors.
// This is the primary transport for CDN-based bypass (ArvanCloud).

async fn ws_upgrade(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_websocket(socket, state))
}

async fn handle_websocket(socket: WebSocket, state: Arc<AppState>) {
    let (mut ws_sender, mut ws_receiver) = socket.split();
    info!("[PHANTOM] WS: new connection, waiting for auth...");

    // ── Step 1: Authenticate via first message ──
    let auth_msg = match tokio::time::timeout(
        Duration::from_secs(10),
        ws_receiver.next(),
    )
    .await
    {
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
            warn!("[PHANTOM] WS auth: invalid JSON: {} — raw: '{}'", e, &auth_msg[..auth_msg.len().min(100)]);
            let _ = ws_sender
                .send(Message::Text(r#"{"error":"invalid request"}"#.into()))
                .await;
            return;
        }
    };

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let drift = if auth.ts > now {
        auth.ts - now
    } else {
        now - auth.ts
    };

    if drift > state.max_drift {
        warn!("[PHANTOM] WS auth rejected: drift {}s (max {}s) — client clock out of sync", drift, state.max_drift);
        let _ = ws_sender
            .send(Message::Text(r#"{"error":"clock drift too high"}"#.into()))
            .await;
        return;
    }

    if !verify_auth(&state.secret, auth.ts, &auth.sig) {
        warn!("[PHANTOM] WS auth rejected: bad HMAC signature — wrong secret");
        let _ = ws_sender
            .send(Message::Text(r#"{"error":"unauthorized"}"#.into()))
            .await;
        return;
    }

    // Create session
    let token = generate_session_token();
    let session = Arc::new(Session::new());
    state
        .sessions
        .lock()
        .await
        .insert(token.clone(), session.clone());

    let _ = ws_sender
        .send(Message::Text(
            serde_json::json!({"token": &token}).to_string(),
        ))
        .await;

    info!("[PHANTOM] ✓ WS session established: {}...", &token[..16]);

    let key = state.key;

    // ── Step 2: Bidirectional relay ──

    // Sender task: downstream frames → WebSocket
    let session_tx = session.clone();
    let sender_task = tokio::spawn(async move {
        let mut rx = session_tx.downstream_rx.lock().await;
        loop {
            let mut frames = Vec::new();

            // Wait for at least one frame or send keepalive
            tokio::select! {
                result = rx.recv() => {
                    match result {
                        Some(frame) => frames.push(frame),
                        None => break, // Channel closed
                    }
                }
                _ = tokio::time::sleep(Duration::from_secs(25)) => {
                    // Send ping to keep connection alive through CDN/proxy
                    if ws_sender.send(Message::Ping(vec![0x50, 0x48])).await.is_err() {
                        break;
                    }
                    continue;
                }
            }

            // Drain any additional available frames
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
    let receiver_task = tokio::spawn(async move {
        while let Some(msg) = ws_receiver.next().await {
            match msg {
                Ok(Message::Binary(data)) => {
                    match decrypt(&key, &data) {
                        Ok(plaintext) => {
                            match decode_frames(&plaintext) {
                                Ok(frames) => {
                                    for frame in frames {
                                        process_upstream_frame(frame, &session_rx).await;
                                    }
                                }
                                Err(e) => error!("[PHANTOM] Frame decode error: {} ({}B)", e, plaintext.len()),
                            }
                        }
                        Err(e) => error!("[PHANTOM] Decrypt error from client: {} ({}B — key mismatch?)", e, data.len()),
                    }
                }
                Ok(Message::Ping(_)) => {
                    // Pong is sent automatically by axum
                }
                Ok(Message::Close(reason)) => {
                    info!("[PHANTOM] WS closed by client: {:?}", reason);
                    break;
                }
                Err(e) => {
                    warn!("[PHANTOM] WS receive error: {}", e);
                    break;
                }
                _ => {} // Ignore text/pong after auth
            }
        }
    });

    // Wait for either task to finish
    tokio::select! {
        _ = sender_task => {},
        _ = receiver_task => {},
    }

    // Cleanup session
    state.sessions.lock().await.remove(&token);
    info!("[PHANTOM] WS session closed: {}...", &token[..16]);
}

// ─── Shared Frame Processing ───────────────────────────────────
// Used by both HTTP sync and WebSocket handlers.

async fn process_upstream_frame(frame: Frame, session: &Arc<Session>) {
    match frame.cmd {
        Cmd::Connect => {
            let addr = String::from_utf8_lossy(&frame.data).to_string();
            let stream_id = frame.stream_id;
            let tx = session.downstream_tx.clone();
            let session = session.clone();

            info!("Stream {} connecting to {}", stream_id, addr);

            tokio::spawn(async move {
                match TcpStream::connect(&addr).await {
                    Ok(tcp) => {
                        let (mut read_half, write_half) = tcp.into_split();
                        session
                            .writers
                            .lock()
                            .await
                            .insert(stream_id, write_half);

                        // Send connect OK
                        let _ = tx.send(Frame::connect_ok(stream_id)).await;
                        info!("Stream {} connected to {}", stream_id, addr);

                        // Spawn reader: TCP → downstream channel
                        let tx2 = tx.clone();
                        tokio::spawn(async move {
                            let mut buf = vec![0u8; 16384];
                            loop {
                                match read_half.read(&mut buf).await {
                                    Ok(0) => {
                                        let _ =
                                            tx2.send(Frame::close(stream_id)).await;
                                        break;
                                    }
                                    Ok(n) => {
                                        let _ = tx2
                                            .send(Frame::data(
                                                stream_id,
                                                buf[..n].to_vec(),
                                            ))
                                            .await;
                                    }
                                    Err(e) => {
                                        error!(
                                            "Stream {} read error: {}",
                                            stream_id, e
                                        );
                                        let _ =
                                            tx2.send(Frame::close(stream_id)).await;
                                        break;
                                    }
                                }
                            }
                        });
                    }
                    Err(e) => {
                        error!("Stream {} connect failed: {}", stream_id, e);
                        let _ = tx
                            .send(Frame::connect_err(stream_id, &e.to_string()))
                            .await;
                    }
                }
            });
        }

        Cmd::Data => {
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
            session.writers.lock().await.remove(&frame.stream_id);
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

    // Decrypt and decode upstream frames
    let encrypted = match b64_decode(&req.d) {
        Ok(d) => d,
        Err(_) => {
            return (StatusCode::OK, Json(serde_json::json!({"d": ""})));
        }
    };

    let plaintext = match decrypt(&state.key, &encrypted) {
        Ok(p) => p,
        Err(_) => {
            return (StatusCode::OK, Json(serde_json::json!({"d": ""})));
        }
    };

    let upstream_frames = decode_frames(&plaintext).unwrap_or_default();

    // Process upstream frames using shared function
    for frame in upstream_frames {
        process_upstream_frame(frame, &session).await;
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
    let encrypted = encrypt(&state.key, &plaintext);
    let encoded = b64_encode(&encrypted);

    (StatusCode::OK, Json(serde_json::json!({"d": encoded})))
}
