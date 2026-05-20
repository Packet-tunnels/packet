// quic_listener.rs — UDP/QUIC server tunnel.
//
// Mirrors `handle_obfs_conn` exactly: same authentication
// (`validate_transport_auth`), same `Session`, same
// `process_upstream_frame_with_relay`. The transport wrapper is just
// quinn over UDP instead of `ObfsStream` over TCP.
//
// We self-sign a cert at startup because the phantom protocol inside the
// QUIC stream authenticates and encrypts on its own — the QUIC TLS layer
// is camouflage to look like a real HTTP/3 / WhatsApp / Meet connection,
// not the security boundary. Clients are configured with a permissive
// verifier (see `packet-client/src/quic_tunnel.rs`), so cert chains and
// hostnames don't matter at this layer.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use phantom_proto::{decode_frames, decrypt, encode_frames, encrypt, AuthRequest};
use quinn::crypto::rustls::QuicServerConfig;
use quinn::{Endpoint, ServerConfig};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use tracing::{debug, error, info, warn};

use crate::{
    generate_session_token, process_upstream_frame_with_relay, validate_transport_auth, AppState,
    Session,
};

/// ALPN announced in the QUIC ServerHello. We accept `h3` (client side
/// sends only `h3`), plus a couple of plausible-looking application
/// protocols so a non-h3 client wouldn't see an immediate ALPN reject.
const QUIC_ALPN_TOKENS: &[&[u8]] = &[b"h3", b"phantom"];

/// Generate a fresh ECDSA P-256 self-signed cert at startup. quinn requires
/// a server cert; the client never validates it. Re-running the server
/// gives a new cert — that's fine, sessions don't survive a restart.
fn generate_self_signed() -> Result<(CertificateDer<'static>, PrivateKeyDer<'static>), String> {
    let cert = rcgen::generate_simple_self_signed(vec!["packet".to_string()])
        .map_err(|e| format!("rcgen: {}", e))?;
    let cert_der = CertificateDer::from(cert.cert.der().to_vec());
    let key_der = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(
        cert.key_pair.serialize_der(),
    ));
    Ok((cert_der, key_der))
}

fn build_server_config() -> Result<ServerConfig, String> {
    let (cert, key) = generate_self_signed()?;
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut tls = rustls::ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|e| format!("server rustls init: {}", e))?
        .with_no_client_auth()
        .with_single_cert(vec![cert], key)
        .map_err(|e| format!("server cert install: {}", e))?;
    tls.alpn_protocols = QUIC_ALPN_TOKENS.iter().map(|p| p.to_vec()).collect();
    let quic_tls =
        QuicServerConfig::try_from(tls).map_err(|e| format!("quic server tls: {}", e))?;
    let mut cfg = ServerConfig::with_crypto(Arc::new(quic_tls));
    let mut transport = quinn::TransportConfig::default();
    transport.max_idle_timeout(Some(
        Duration::from_secs(60).try_into().map_err(|e| format!("idle: {}", e))?,
    ));
    transport.max_concurrent_bidi_streams(2048u32.into());
    transport.max_concurrent_uni_streams(64u32.into());
    cfg.transport_config(Arc::new(transport));
    Ok(cfg)
}

/// Spawn the QUIC listener task. Binds dual-stack `[::]:port` (matches the
/// HTTP/WS listener style in `main.rs`) so v4 and v6 clients land on the
/// same UDP port.
pub async fn run_quic_listener(port: u16, state: Arc<AppState>) {
    let cfg = match build_server_config() {
        Ok(c) => c,
        Err(e) => {
            error!("[PHANTOM] QUIC server config failed: {}", e);
            return;
        }
    };
    let addr: SocketAddr = match format!("[::]:{}", port).parse() {
        Ok(a) => a,
        Err(e) => {
            error!("[PHANTOM] QUIC listen addr parse failed: {}", e);
            return;
        }
    };
    let endpoint = match Endpoint::server(cfg, addr) {
        Ok(ep) => ep,
        Err(e) => {
            // Dual-stack bind can fail on hosts without v6 — fall back to v4.
            warn!("[PHANTOM] QUIC dual-stack bind on {} failed ({}); falling back to IPv4-only", addr, e);
            let v4: SocketAddr = format!("0.0.0.0:{}", port).parse().unwrap();
            match Endpoint::server(build_server_config().unwrap(), v4) {
                Ok(ep) => ep,
                Err(e2) => {
                    error!("[PHANTOM] QUIC IPv4 fallback bind failed: {}", e2);
                    return;
                }
            }
        }
    };
    info!(
        "[PHANTOM] QUIC tunnel listener LIVE on {} (UDP, dual-stack)",
        endpoint.local_addr().map(|a| a.to_string()).unwrap_or_default()
    );

    while let Some(connecting) = endpoint.accept().await {
        let state = state.clone();
        tokio::spawn(async move {
            match connecting.await {
                Ok(conn) => {
                    let peer = conn.remote_address();
                    if let Err(e) = handle_quic_conn(conn, state).await {
                        debug!("[PHANTOM] QUIC session from {} ended: {}", peer, e);
                    }
                }
                Err(e) => warn!("[PHANTOM] QUIC handshake failed: {}", e),
            }
        });
    }
}

async fn handle_quic_conn(
    conn: quinn::Connection,
    state: Arc<AppState>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let peer = conn.remote_address();
    debug!("[PHANTOM] QUIC connection from {}", peer);

    // ── Auth: first bidirectional stream is one length-delimited JSON ─
    let (mut auth_send, mut auth_recv) = tokio::time::timeout(
        Duration::from_secs(10),
        conn.accept_bi(),
    )
    .await
    .map_err(|_| "QUIC auth: client did not open auth stream in 10s")??;
    let auth_bytes = read_quic_msg(&mut auth_recv).await?;
    let auth: AuthRequest = serde_json::from_slice(&auth_bytes)?;

    let validated = match validate_transport_auth(&state, &auth).await {
        Ok(v) => v,
        Err(error) => {
            warn!("[PHANTOM] QUIC auth rejected: {}", error);
            let _ = write_quic_msg(&mut auth_send, br#"{"error":"unauthorized"}"#).await;
            let _ = auth_send.finish();
            return Ok(());
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

    let resp = serde_json::json!({
        "token": &token,
        "relay": has_relay,
        "auth_mode": validated.mode,
        "bridge_id": validated.bridge_id,
    })
    .to_string();
    write_quic_msg(&mut auth_send, resp.as_bytes()).await?;
    let _ = auth_send.finish();
    info!(
        "[PHANTOM] ✓ QUIC session established: {}… peer={} (relay: {}, mode: {})",
        &token[..token.len().min(16)],
        peer,
        has_relay,
        validated.mode
    );

    let key = session.key;

    // ── Sender task: downstream frames → new bidi streams ─────────
    let send_conn = conn.clone();
    let session_tx = session.clone();
    let sender = tokio::spawn(async move {
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
                    // Keepalive: open a stream, write an empty payload,
                    // close it. QUIC keep-alive also runs at transport
                    // level (set in client + server transport config).
                    if let Ok((mut s, _)) = send_conn.open_bi().await {
                        let _ = write_quic_msg(&mut s, &[]).await;
                        let _ = s.finish();
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
            if frames.is_empty() {
                continue;
            }
            let plaintext = encode_frames(&frames);
            let encrypted = encrypt(&key, &plaintext);
            match send_conn.open_bi().await {
                Ok((mut s, _r)) => {
                    if write_quic_msg(&mut s, &encrypted).await.is_err() {
                        break;
                    }
                    let _ = s.finish();
                }
                Err(_) => break,
            }
        }
    });

    // ── Receiver task: incoming bidi streams → frame routing ──────
    let recv_conn = conn.clone();
    let session_rx = session.clone();
    let rx_state = state.clone();
    let receiver = tokio::spawn(async move {
        loop {
            match recv_conn.accept_bi().await {
                Ok((_send, mut recv)) => match read_quic_msg(&mut recv).await {
                    Ok(payload) => {
                        if payload.is_empty() {
                            continue;
                        }
                        match decrypt(&key, &payload) {
                            Ok(plaintext) => match decode_frames(&plaintext) {
                                Ok(frames) => {
                                    for frame in frames {
                                        process_upstream_frame_with_relay(
                                            frame,
                                            &session_rx,
                                            &rx_state,
                                        )
                                        .await;
                                    }
                                }
                                Err(e) => error!("[PHANTOM] QUIC frame decode: {}", e),
                            },
                            Err(e) => {
                                error!("[PHANTOM] QUIC decrypt error: {} (key mismatch?)", e);
                                break;
                            }
                        }
                    }
                    Err(e) => {
                        debug!("[PHANTOM] QUIC inbound stream read: {}", e);
                    }
                },
                Err(_) => break,
            }
        }
    });

    tokio::select! {
        _ = sender => {}
        _ = receiver => {}
    }
    state.sessions.lock().await.remove(&token);
    let _ = conn.close(0u32.into(), b"server close");
    info!(
        "[PHANTOM] QUIC session closed: {}… peer={}",
        &token[..token.len().min(16)],
        peer
    );
    Ok(())
}

async fn write_quic_msg(
    stream: &mut quinn::SendStream,
    payload: &[u8],
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let len = (payload.len() as u32).to_le_bytes();
    stream.write_all(&len).await?;
    stream.write_all(payload).await?;
    Ok(())
}

async fn read_quic_msg(
    stream: &mut quinn::RecvStream,
) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let len = u32::from_le_bytes(len_buf) as usize;
    if len > 16 * 1024 * 1024 {
        return Err("quic incoming length cap exceeded".into());
    }
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf).await?;
    Ok(buf)
}
