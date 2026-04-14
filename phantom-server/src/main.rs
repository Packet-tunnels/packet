// phantom-server: Covert HTTP tunnel server
//
// This server does two things simultaneously:
// 1. Serves a real static website (piano lessons) to look legitimate
// 2. Provides authenticated tunnel API endpoints for Phantom clients
//
// The tunnel data is hidden inside normal-looking JSON API calls.
// To censors and probes, this looks like a standard web application.

use axum::{
    extract::State,
    http::StatusCode,
    response::{Html, IntoResponse},
    routing::{get, post},
    Json, Router,
};
use clap::Parser;
use phantom_proto::*;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::net::tcp::OwnedWriteHalf;
use tokio::sync::{mpsc, Mutex};
use tracing::{error, info, warn};

// ─── CLI ───────────────────────────────────────────────────────
#[derive(Parser)]
#[command(name = "phantom-server")]
#[command(about = "Phantom Tunnel Server — covert HTTP tunnel")]
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
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();
    let key = derive_key(&cli.secret);

    info!("Phantom Server starting on port {}", cli.port);

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
            tokio::time::sleep(tokio::time::Duration::from_secs(300)).await;
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

// ─── Tunnel Sync ───────────────────────────────────────────────

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

    // Process upstream frames
    for frame in upstream_frames {
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
                            session.writers.lock().await.insert(stream_id, write_half);

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
                                                tx2.send(Frame::close(stream_id))
                                                    .await;
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
                                                tx2.send(Frame::close(stream_id))
                                                    .await;
                                            break;
                                        }
                                    }
                                }
                            });
                        }
                        Err(e) => {
                            error!(
                                "Stream {} connect failed: {}",
                                stream_id, e
                            );
                            let _ = tx
                                .send(Frame::connect_err(
                                    stream_id,
                                    &e.to_string(),
                                ))
                                .await;
                        }
                    }
                });
            }

            Cmd::Data => {
                let mut writers = session.writers.lock().await;
                if let Some(writer) = writers.get_mut(&frame.stream_id) {
                    if let Err(e) = writer.write_all(&frame.data).await {
                        error!(
                            "Stream {} write error: {}",
                            frame.stream_id, e
                        );
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

    // Give spawned tasks a moment to produce downstream data
    tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;

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
