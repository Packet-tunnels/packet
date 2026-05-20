// quic_tunnel.rs — UDP/QUIC escape transport.
//
// The Iran filter we are fighting (May 2026) RSTs every TCP-TLS handshake to
// every Cloudflare anycast IP from the network captured in our diagnostics,
// regardless of fingerprint. UDP traffic to UDP/443 is not affected by the
// same path because the carrier needs to pass WhatsApp / Google Meet /
// Telegram video calls — and those run on QUIC or DTLS over UDP/443. By
// running our tunnel inside a real QUIC connection to UDP/443, we look like
// one of those calls and pass the same filter.
//
// Wire-format inside QUIC:
//
//     * Open bidirectional stream 0   (control)
//         client sends one length-delimited authentication message
//             encrypted with the shared `key` exactly like the WS path
//             expects, and reads back the auth response (token + features).
//     * Subsequent bidi streams       (control + downstream frames)
//         server pushes new bidi streams whenever it has downstream frames
//         to deliver; payload is length-delimited encrypted frame batches.
//     * Client opens new bidi streams for every batch of upstream frames.
//
// Reliability and ordering inside one connection are provided by QUIC's
// per-stream byte stream semantics. Frame-level multiplexing of SOCKS
// connections is done by phantom's `stream_id` inside the encoded frames
// (same as the WS path), so the server-side dispatch logic is unchanged.
//
// The phantom protocol authenticates / encrypts on its own (shared
// `key`), so the QUIC layer here is configured with a permissive cert
// verifier — the TLS in QUIC is camouflage, not security.

use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
use std::sync::Arc;
use std::time::{Duration, Instant};

use phantom_proto::*;
use quinn::crypto::rustls::QuicClientConfig;
use quinn::{ClientConfig, Endpoint};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use tokio::sync::{mpsc, Mutex};
use tracing::{debug, error, info, warn};

use crate::transport::{build_transport_auth_request, TransportConfig};
use crate::{
    add_runtime_bytes_down, add_runtime_bytes_up, dispatch_downstream, encode_frames,
    process_upstream_msg, set_runtime_connected, set_runtime_state, TunnelState, UpstreamMsg,
};

/// Wire framing prefix inside a QUIC stream. We do not rely on QUIC's own
/// stream-close to signal message boundaries; instead each stream carries
/// exactly ONE length-prefixed encrypted blob so we can pipeline aggressively
/// without ordering bugs.
const QUIC_MAX_MSG: u32 = 16 * 1024 * 1024;
const QUIC_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const QUIC_AUTH_TIMEOUT: Duration = Duration::from_secs(10);
/// ALPN advertised in the QUIC ClientHello. We use the HTTP/3 token so the
/// handshake looks like a browser opening an HTTP/3 connection.
const QUIC_ALPN: &[u8] = b"h3";

/// Permissive verifier — same role as `chrome_tls::CaptureVerifier`. The
/// phantom layer inside the tunnel handles auth/encryption; QUIC TLS here
/// only provides confidentiality + DPI camouflage.
#[derive(Debug)]
struct PermissiveQuicVerifier;
impl ServerCertVerifier for PermissiveQuicVerifier {
    fn verify_server_cert(
        &self,
        _e: &CertificateDer<'_>,
        _i: &[CertificateDer<'_>],
        _n: &ServerName<'_>,
        _o: &[u8],
        _t: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        _m: &[u8],
        _c: &CertificateDer<'_>,
        _d: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }
    fn verify_tls13_signature(
        &self,
        _m: &[u8],
        _c: &CertificateDer<'_>,
        _d: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![
            SignatureScheme::ECDSA_NISTP256_SHA256,
            SignatureScheme::ECDSA_NISTP384_SHA384,
            SignatureScheme::ED25519,
            SignatureScheme::RSA_PSS_SHA256,
            SignatureScheme::RSA_PSS_SHA384,
            SignatureScheme::RSA_PSS_SHA512,
            SignatureScheme::RSA_PKCS1_SHA256,
            SignatureScheme::RSA_PKCS1_SHA384,
            SignatureScheme::RSA_PKCS1_SHA512,
        ]
    }
}

fn build_quic_client_config() -> Result<ClientConfig, String> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut tls = rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|e| format!("quic rustls init: {}", e))?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(PermissiveQuicVerifier))
        .with_no_client_auth();
    tls.alpn_protocols = vec![QUIC_ALPN.to_vec()];
    let quic_tls = QuicClientConfig::try_from(tls)
        .map_err(|e| format!("quic tls config: {}", e))?;
    let mut cfg = ClientConfig::new(Arc::new(quic_tls));
    let mut transport = quinn::TransportConfig::default();
    // Keep the path warm. 15s is well below most NAT UDP idle timeouts.
    transport.keep_alive_interval(Some(Duration::from_secs(15)));
    transport.max_idle_timeout(Some(Duration::from_secs(60).try_into().map_err(|e| {
        format!("idle timeout: {}", e)
    })?));
    // QUIC stream concurrency — generous to support many parallel SOCKS
    // streams over a single connection.
    transport.max_concurrent_bidi_streams(1024u32.into());
    cfg.transport_config(Arc::new(transport));
    Ok(cfg)
}

/// Resolve `host:port` (host may be IP or DNS) into a SocketAddr. tokio's
/// `lookup_host` returns v6 first when present, which is what we want.
fn resolve(host_port: &str) -> Result<SocketAddr, String> {
    host_port
        .to_socket_addrs()
        .map_err(|e| format!("dns {}: {}", host_port, e))?
        .next()
        .ok_or_else(|| format!("dns {}: no result", host_port))
}

/// Top-level QUIC tunnel loop. Mirrors `run_ws_loop` / `run_http_loop` from
/// `transport.rs` — connects, authenticates, runs the bidirectional frame
/// relay, and reconnects on failure with backoff.
pub async fn run_quic_loop(
    config: TransportConfig,
    mut upstream_rx: mpsc::Receiver<UpstreamMsg>,
    tunnel_state: Arc<Mutex<TunnelState>>,
) {
    let mut backoff = Duration::from_millis(500);
    loop {
        match quic_session(&config, &mut upstream_rx, &tunnel_state).await {
            Ok(()) => {
                info!("[PHANTOM] QUIC session ended gracefully, reconnecting in 500ms…");
                backoff = Duration::from_millis(500);
                tokio::time::sleep(backoff).await;
            }
            Err(error) => {
                warn!("[PHANTOM] ❌ QUIC FAILED: {} | retry in {:?}", error, backoff);
                set_runtime_state("reconnecting");
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(Duration::from_secs(30));
            }
        }
    }
}

/// One-attempt connect-and-run used by the rotation supervisor in
/// `transport.rs::run_rotating_transport`. The Ok variant means the QUIC
/// session ran to completion (no per-attempt failure to retry on); Err is a
/// connect / auth / runtime failure that should trigger candidate rotation.
pub(crate) async fn quic_session_once(
    config: &TransportConfig,
    upstream_rx: &mut mpsc::Receiver<UpstreamMsg>,
    tunnel_state: &Arc<Mutex<TunnelState>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    quic_session(config, upstream_rx, tunnel_state).await
}

async fn quic_session(
    config: &TransportConfig,
    upstream_rx: &mut mpsc::Receiver<UpstreamMsg>,
    tunnel_state: &Arc<Mutex<TunnelState>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let started_at = Instant::now();
    let target = config.connect_addr();
    let server_sni = config.sni_value();
    let key = config.key;

    info!("[PHANTOM] QUIC connecting target={} sni={}", target, server_sni);

    // Bind the local endpoint. Use a wildcard that matches the destination
    // family so v4 traffic goes out a v4 socket and v6 out a v6 socket.
    let remote = resolve(&target)?;
    let local: SocketAddr = match remote.ip() {
        IpAddr::V4(_) => "0.0.0.0:0".parse().unwrap(),
        IpAddr::V6(_) => "[::]:0".parse().unwrap(),
    };
    let mut endpoint = Endpoint::client(local)
        .map_err(|e| format!("QUIC endpoint bind: {}", e))?;
    endpoint.set_default_client_config(build_quic_client_config()?);

    // QUIC connect with a hard timeout so a dead UDP path fails fast.
    let connecting = endpoint.connect(remote, &server_sni)
        .map_err(|e| format!("QUIC connect setup: {}", e))?;
    let connection = tokio::time::timeout(QUIC_HANDSHAKE_TIMEOUT, connecting)
        .await
        .map_err(|_| "QUIC handshake timed out (UDP path blocked?)")?
        .map_err(|e| format!("QUIC handshake: {}", e))?;
    info!(
        "[PHANTOM] ✓ QUIC handshake complete in {}ms (rtt {}ms)",
        started_at.elapsed().as_millis(),
        connection.rtt().as_millis()
    );

    // ── Auth on the first bidirectional stream ────────────────────
    let (mut auth_send, mut auth_recv) = connection
        .open_bi()
        .await
        .map_err(|e| format!("QUIC open auth stream: {}", e))?;

    let auth_request = build_transport_auth_request(config, Some("mesh_client"));
    let auth_json = serde_json::to_vec(&auth_request)?;
    write_quic_msg(&mut auth_send, &auth_json).await?;
    auth_send
        .finish()
        .map_err(|e| format!("QUIC auth finish: {}", e))?;

    let auth_resp = tokio::time::timeout(QUIC_AUTH_TIMEOUT, read_quic_msg(&mut auth_recv))
        .await
        .map_err(|_| "QUIC auth: server did not respond in 10s")??;
    let auth_value: serde_json::Value = serde_json::from_slice(&auth_resp)?;
    if let Some(err) = auth_value.get("error") {
        return Err(format!("QUIC auth REJECTED: {}", err).into());
    }
    let token = auth_value["token"].as_str().unwrap_or("unknown").to_string();
    info!(
        "[PHANTOM] ✓ QUIC authenticated session={}…",
        &token[..token.len().min(16)]
    );
    set_runtime_state("connected");
    let ping_ms = started_at.elapsed().as_millis().min(u32::MAX as u128) as u32;
    set_runtime_connected(Some(ping_ms));

    // ── Downstream reader task ────────────────────────────────────
    // QUIC's `accept_bi` is cancel-safe (it just polls the connection
    // event queue), but `read_quic_msg` over the resulting stream is not
    // — so we own that read loop in a dedicated task and forward decoded
    // payload bytes through a cancel-safe mpsc into the main relay.
    let (down_tx, mut down_rx) = mpsc::channel::<Result<Vec<u8>, String>>(256);
    let down_conn = connection.clone();
    let reader = tokio::spawn(async move {
        loop {
            match down_conn.accept_bi().await {
                Ok((_send, mut recv)) => match read_quic_msg(&mut recv).await {
                    Ok(payload) => {
                        if down_tx.send(Ok(payload)).await.is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        let _ = down_tx.send(Err(format!("downstream read: {}", e))).await;
                        break;
                    }
                },
                Err(e) => {
                    let _ = down_tx
                        .send(Err(format!("downstream accept_bi: {}", e)))
                        .await;
                    break;
                }
            }
        }
    });

    info!("[PHANTOM] ✓ QUIC TUNNEL ACTIVE — relay is live (target {})", target);

    // ── Bidirectional relay loop ──────────────────────────────────
    let outcome: Result<(), Box<dyn std::error::Error + Send + Sync>> = loop {
        tokio::select! {
            biased;
            // ── Downstream: server → client ───────────────────────
            framed = down_rx.recv() => {
                let payload = match framed {
                    Some(Ok(b)) => b,
                    Some(Err(e)) => break Err(e.into()),
                    None => break Err("QUIC reader task ended".into()),
                };
                if payload.is_empty() { continue; }
                add_runtime_bytes_down(payload.len() as u64);
                match decrypt(&key, &payload) {
                    Ok(plaintext) => match decode_frames(&plaintext) {
                        Ok(frames) => {
                            let app_frames: Vec<_> = frames
                                .into_iter()
                                .filter(|f| f.cmd != Cmd::Relay)
                                .collect();
                            dispatch_downstream(app_frames, tunnel_state).await;
                        }
                        Err(e) => error!("[PHANTOM] QUIC frame decode error: {}", e),
                    },
                    Err(e) => error!("[PHANTOM] QUIC decrypt failed: {}", e),
                }
            }
            // ── Upstream: client → server ─────────────────────────
            msg = upstream_rx.recv() => {
                match msg {
                    Some(msg) => {
                        let mut frames = Vec::new();
                        process_upstream_msg(msg, &mut frames, tunnel_state).await;
                        while let Ok(msg) = upstream_rx.try_recv() {
                            process_upstream_msg(msg, &mut frames, tunnel_state).await;
                        }
                        if frames.is_empty() { continue; }
                        let plaintext = encode_frames(&frames);
                        let encrypted = encrypt(&key, &plaintext);
                        add_runtime_bytes_up(encrypted.len() as u64);
                        // Each batch goes on a fresh bidi stream — QUIC
                        // multiplexes them in parallel without head-of-line
                        // blocking between batches.
                        match connection.open_bi().await {
                            Ok((mut send, _recv)) => {
                                if let Err(e) = write_quic_msg(&mut send, &encrypted).await {
                                    break Err(format!("QUIC upstream write: {}", e).into());
                                }
                                if let Err(e) = send.finish() {
                                    debug!("[PHANTOM] QUIC stream finish: {}", e);
                                }
                            }
                            Err(e) => {
                                break Err(format!("QUIC open_bi: {}", e).into());
                            }
                        }
                    }
                    None => {
                        info!("[PHANTOM] QUIC upstream channel closed");
                        break Ok(());
                    }
                }
            }
        }
    };

    reader.abort();
    let _ = connection.close(0u32.into(), b"client close");
    endpoint.wait_idle().await;
    outcome
}

async fn write_quic_msg(
    stream: &mut quinn::SendStream,
    payload: &[u8],
) -> Result<(), String> {
    if payload.len() as u64 > QUIC_MAX_MSG as u64 {
        return Err("quic message exceeds max size".to_string());
    }
    let len = (payload.len() as u32).to_le_bytes();
    stream
        .write_all(&len)
        .await
        .map_err(|e| format!("write len: {}", e))?;
    stream
        .write_all(payload)
        .await
        .map_err(|e| format!("write payload: {}", e))?;
    Ok(())
}

async fn read_quic_msg(stream: &mut quinn::RecvStream) -> Result<Vec<u8>, String> {
    let mut len_buf = [0u8; 4];
    stream
        .read_exact(&mut len_buf)
        .await
        .map_err(|e| format!("read len: {}", e))?;
    let len = u32::from_le_bytes(len_buf);
    if len > QUIC_MAX_MSG {
        return Err(format!("quic incoming length {} exceeds cap", len));
    }
    let mut buf = vec![0u8; len as usize];
    stream
        .read_exact(&mut buf)
        .await
        .map_err(|e| format!("read payload: {}", e))?;
    Ok(buf)
}
