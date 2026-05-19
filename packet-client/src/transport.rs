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

use crate::{
    dispatch_downstream, mesh, process_upstream_msg, set_runtime_connected, set_runtime_last_error,
    set_runtime_ping, set_runtime_state, tls_fragment::FragmentStream, TunnelState, UpstreamMsg,
};
use futures_util::{SinkExt, StreamExt};
use phantom_proto::*;
use reqwest::header::HOST;
use reqwest::Client;
use std::error::Error as _;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, Mutex};
use tokio_rustls::rustls::client::danger::{
    HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier,
};
use tokio_rustls::rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use tokio_rustls::rustls::{
    client::WebPkiServerVerifier, CertificateError, ClientConfig as RustlsClientConfig,
    DigitallySignedStruct, RootCertStore, SignatureScheme,
};
use tokio_rustls::TlsConnector;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;
use tracing::{debug, error, info, warn};
use webpki::EndEntityCert;

// ─── Transport Configuration ──────────────────────────────────

#[derive(Clone, Debug)]
pub enum TransportMode {
    Http,
    WebSocket,
    Auto,
    /// Browser-like HTTPS POST transport for stricter DPI environments.
    Stealth,
    /// Raw-TCP OSSH-style obfuscated transport. No TLS ClientHello, no HTTP,
    /// no SNI — the wire is uniform random from byte 0. Designed to slip
    /// past Iran's "RST any foreign TLS handshake" filter. Connects to a
    /// directly-reachable foreign IP:port (passed via `cdn_edge`), NOT a
    /// TLS-terminating CDN.
    Obfs,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TlsProfile {
    Default,
    BrowserLike,
}

#[derive(Clone, Debug)]
pub struct TransportConfig {
    pub server_url: String,
    pub secret: String,
    pub auth_ticket: Option<String>,
    pub key: [u8; 32],
    pub mode: TransportMode,
    /// Custom Host header for CDN mode (e.g., "piano-lessons.site")
    pub host_header: Option<String>,
    /// CDN edge IP to connect to (e.g., "185.143.234.235:80")
    pub cdn_edge: Option<String>,
    /// Custom SNI for TLS ClientHello to bypass DPI filtering
    pub sni_override: Option<String>,
    /// Enable TLS ClientHello fragmentation
    pub fragment_enabled: bool,
    /// Fragment chunk size in bytes
    pub fragment_size: usize,
    /// SPKI SHA-256 pins for the selected bridge descriptor
    pub spki_pins: Vec<String>,
    /// TLS and HTTP header shaping profile.
    pub tls_profile: TlsProfile,
    /// Optional lane/profile-specific ALPN override.
    pub alpn_override: Option<Vec<Vec<u8>>>,
    /// Optional lane/profile-specific User-Agent override.
    pub user_agent_override: Option<String>,
    /// Pre-shared obfuscation "knock" for `TransportMode::Obfs`. Must match
    /// the server's `--obfs-key`. Low-entropy is fine — the real crypto is
    /// the inner frame layer.
    pub obfs_key: Option<String>,
    /// Optional first-hop proxy used to reach the transport ingress. This is
    /// the Psiphon/Conduit-style layer for networks where the foreign IP is
    /// reachable only through a local or private bridge.
    pub upstream_proxy: Option<String>,
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

    /// Hostname to place in the HTTP URL itself.
    /// In CDN TLS mode this controls the TLS SNI and must stay on the fronting host,
    /// while DNS is overridden to the edge IP separately.
    fn http_request_host(&self) -> String {
        if self.cdn_edge.is_some() {
            if self.is_tls() {
                self.sni_value()
            } else {
                self.host_value()
            }
        } else {
            url::Url::parse(&self.server_url)
                .ok()
                .and_then(|u| u.host_str().map(|s| s.to_string()))
                .unwrap_or_else(|| self.host_value())
        }
    }

    fn http_request_port(&self) -> Option<u16> {
        if self.cdn_edge.is_some() {
            self.connect_addr()
                .parse::<SocketAddr>()
                .ok()
                .map(|socket_addr| socket_addr.port())
        } else {
            None
        }
    }

    fn http_resolve_override(&self) -> Option<(String, SocketAddr)> {
        self.cdn_edge.as_ref()?;
        let request_host = self.http_request_host();
        let connect_addr = self.connect_addr().parse::<SocketAddr>().ok()?;
        Some((request_host, connect_addr))
    }

    /// Get the HTTP endpoint URL to send requests to.
    /// CDN mode: keep the URL on the fronting hostname, and override DNS to the edge IP.
    /// Direct mode: use the configured server URL as-is.
    fn http_request_url(&self, path: &str) -> String {
        let normalized_path = if path.starts_with('/') {
            path.to_string()
        } else {
            format!("/{}", path)
        };

        if self.cdn_edge.is_some() {
            let scheme = if self.is_tls() { "https" } else { "http" };
            let request_host = self.http_request_host();
            let default_port = if self.is_tls() { 443 } else { 80 };
            let authority = match self.http_request_port() {
                Some(port) if port != default_port => format!("{}:{}", request_host, port),
                _ => request_host,
            };
            format!("{}://{}{}", scheme, authority, normalized_path)
        } else {
            format!(
                "{}{}",
                self.server_url.trim_end_matches('/'),
                normalized_path
            )
        }
    }

    /// Host header for HTTP requests.
    /// In CDN mode this must stay on the origin-looking host, not the CDN IP.
    fn http_host_header(&self) -> Option<String> {
        if self.cdn_edge.is_some() || self.host_header.is_some() {
            Some(self.host_value())
        } else {
            None
        }
    }

    /// Get the SNI to use for TLS handshakes.
    fn sni_value(&self) -> String {
        if let Some(ref sni) = self.sni_override {
            sni.clone()
        } else {
            self.host_value()
        }
    }

    fn uses_browser_like_tls(&self) -> bool {
        self.tls_profile == TlsProfile::BrowserLike || matches!(self.mode, TransportMode::Stealth)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TransportUpstreamAuth {
    username: String,
    password: String,
}

impl TransportUpstreamAuth {
    fn from_url(url: &url::Url) -> Option<Self> {
        let username = url.username().trim();
        if username.is_empty() {
            return None;
        }
        Some(Self {
            username: username.to_string(),
            password: url.password().unwrap_or("").to_string(),
        })
    }

    fn basic_header_value(&self) -> String {
        use base64::Engine as _;

        let raw = format!("{}:{}", self.username, self.password);
        format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode(raw)
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum TransportUpstreamProxy {
    Socks5 {
        host: String,
        port: u16,
        auth: Option<TransportUpstreamAuth>,
    },
    Http {
        host: String,
        port: u16,
        auth: Option<TransportUpstreamAuth>,
    },
}

impl TransportUpstreamProxy {
    fn parse(raw: &str) -> Result<Self, String> {
        let raw = raw.trim();
        if raw.is_empty() {
            return Err("upstream proxy is empty".to_string());
        }

        let url = url::Url::parse(raw)
            .map_err(|error| format!("invalid upstream proxy URI: {}", error))?;
        let host = url
            .host_str()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| "upstream proxy is missing a host".to_string())?
            .to_string();
        let port = url
            .port()
            .ok_or_else(|| "upstream proxy is missing a port".to_string())?;
        let auth = TransportUpstreamAuth::from_url(&url);

        match url.scheme().to_ascii_lowercase().as_str() {
            "socks" | "socks5" => Ok(Self::Socks5 { host, port, auth }),
            "http" | "https" => Ok(Self::Http { host, port, auth }),
            other => Err(format!(
                "upstream proxy must be socks5://host:port or http://host:port, got {}",
                other
            )),
        }
    }

    fn connect_addr(&self) -> String {
        match self {
            Self::Socks5 { host, port, .. } | Self::Http { host, port, .. } => {
                format!("{}:{}", host, port)
            }
        }
    }

    fn label(&self) -> &'static str {
        match self {
            Self::Socks5 { .. } => "socks5",
            Self::Http { .. } => "http",
        }
    }
}

#[derive(Debug)]
struct PinnedServerCertVerifier {
    inner: Arc<WebPkiServerVerifier>,
    spki_pins: Vec<String>,
}

impl PinnedServerCertVerifier {
    fn new(inner: Arc<WebPkiServerVerifier>, pins: &[String]) -> Result<Self, String> {
        let spki_pins = normalize_spki_pins(pins);
        if spki_pins.is_empty() {
            return Err("packet bridge SPKI pins missing".to_string());
        }

        Ok(Self { inner, spki_pins })
    }

    fn verify_spki_pin(
        &self,
        end_entity: &CertificateDer<'_>,
    ) -> Result<(), tokio_rustls::rustls::Error> {
        let parsed = EndEntityCert::try_from(end_entity).map_err(|_| {
            tokio_rustls::rustls::Error::InvalidCertificate(CertificateError::BadEncoding)
        })?;
        let actual_pin = b64_encode(&sha256(parsed.subject_public_key_info().as_ref()));

        if self
            .spki_pins
            .iter()
            .any(|expected| expected == &actual_pin)
        {
            Ok(())
        } else {
            Err(tokio_rustls::rustls::Error::InvalidCertificate(
                CertificateError::ApplicationVerificationFailure,
            ))
        }
    }
}

impl ServerCertVerifier for PinnedServerCertVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, tokio_rustls::rustls::Error> {
        self.inner.verify_server_cert(
            end_entity,
            intermediates,
            server_name,
            ocsp_response,
            now,
        )?;
        self.verify_spki_pin(end_entity)?;
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, tokio_rustls::rustls::Error> {
        self.inner.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, tokio_rustls::rustls::Error> {
        self.inner.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.inner.supported_verify_schemes()
    }
}

fn normalize_spki_pins(pins: &[String]) -> Vec<String> {
    let mut normalized = Vec::new();
    for pin in pins {
        let trimmed = pin.trim();
        if trimmed.is_empty() {
            continue;
        }

        let value = trimmed
            .strip_prefix("sha256/")
            .or_else(|| trimmed.strip_prefix("SHA256/"))
            .unwrap_or(trimmed)
            .trim();

        if !value.is_empty() && !normalized.iter().any(|existing| existing == value) {
            normalized.push(value.to_string());
        }
    }
    normalized
}

fn apply_tls_profile(
    mut tls_config: RustlsClientConfig,
    config: &TransportConfig,
) -> RustlsClientConfig {
    if let Some(alpn) = config.alpn_override.as_ref() {
        tls_config.alpn_protocols = alpn.clone();
    } else if config.uses_browser_like_tls() {
        // This is not an exact Chrome/uTLS ClientHello, but it matches the
        // critical browser ALPN set used by fronted HTTPS POST transports.
        tls_config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    }
    tls_config
}

fn build_tls_client_config(config: &TransportConfig) -> Result<RustlsClientConfig, String> {
    let mut root_store = RootCertStore::empty();
    root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

    if config.spki_pins.is_empty() {
        let tls_config = RustlsClientConfig::builder()
            .with_root_certificates(root_store)
            .with_no_client_auth();
        return Ok(apply_tls_profile(tls_config, config));
    }

    let verifier = WebPkiServerVerifier::builder(Arc::new(root_store))
        .build()
        .map_err(|error| format!("packet bridge verifier init failed: {}", error))?;
    let verifier = Arc::new(PinnedServerCertVerifier::new(verifier, &config.spki_pins)?);

    let tls_config = RustlsClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();
    Ok(apply_tls_profile(tls_config, config))
}

// ─── Transport Runner ──────────────────────────────────────────

pub async fn run_transport(
    config: TransportConfig,
    upstream_rx: mpsc::Receiver<UpstreamMsg>,
    tunnel_state: Arc<Mutex<TunnelState>>,
) {
    match config.mode {
        // WebSocket / Obfs / Auto are the Iran escape paths — run them under
        // the Psiphon-style candidate-rotation supervisor instead of the
        // single-config retry loop, so we grind a whole pool until one
        // combination punches through (this is *why* the working stacks take
        // minutes to connect, not seconds).
        TransportMode::WebSocket | TransportMode::Obfs | TransportMode::Auto => {
            run_rotating_transport(config, upstream_rx, tunnel_state).await;
        }
        TransportMode::Http => {
            run_http_loop(config, upstream_rx, tunnel_state).await;
        }
        TransportMode::Stealth => {
            run_stealth_loop(config, upstream_rx, tunnel_state).await;
        }
    }
}

/// Psiphon-style persistent multi-candidate connector.
///
/// Walks the candidate pool forever. For each candidate it makes ONE attempt
/// (`ws_session`/`obfs_session` connect + run-until-drop). On failure it
/// advances to the next candidate; on success it keeps that candidate first
/// next time. It never reports "failed" — only "connecting" with progress —
/// because the working reference stacks legitimately take up to ~10 minutes
/// of grinding before a path opens.
async fn run_rotating_transport(
    base: TransportConfig,
    mut upstream_rx: mpsc::Receiver<UpstreamMsg>,
    tunnel_state: Arc<Mutex<TunnelState>>,
) {
    let mut pool = crate::candidates::build_candidates(&base);
    if pool.is_empty() {
        warn!("[PHANTOM] candidate pool empty, falling back to base WS loop");
        run_ws_loop(base, upstream_rx, tunnel_state).await;
        return;
    }

    info!(
        "[PHANTOM] persistent connector: {} candidates, grinding until one opens",
        pool.len()
    );

    let mut idx = 0usize;
    let mut attempt: u64 = 0;
    loop {
        let n = pool.len();
        let cand_idx = idx % n;
        attempt += 1;
        set_runtime_state("connecting");
        info!(
            "[PHANTOM] connect attempt #{} — candidate {}/{}: {}",
            attempt,
            cand_idx + 1,
            n,
            pool[cand_idx].label
        );

        let cfg = pool[cand_idx].config.clone();
        let result = match cfg.mode {
            TransportMode::Obfs => obfs_session(&cfg, &mut upstream_rx, &tunnel_state).await,
            _ => ws_session(&cfg, &mut upstream_rx, &tunnel_state).await,
        };

        match result {
            Ok(()) => {
                // Session ran and ended (clean drop or upstream channel
                // closed). Keep this candidate first so a reconnect reuses
                // the known-good path immediately.
                info!(
                    "[PHANTOM] candidate '{}' session ended; will retry it first",
                    pool[cand_idx].label
                );
                if cand_idx != 0 {
                    let good = pool.remove(cand_idx);
                    pool.insert(0, good);
                }
                idx = 0;
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            Err(e) => {
                warn!(
                    "[PHANTOM] candidate '{}' failed: {} — rotating",
                    pool[cand_idx].label, e
                );
                idx = idx.wrapping_add(1);
                // Short gap between candidates; a longer breather after a
                // full sweep so we don't hammer a hostile network.
                let swept = idx % n == 0;
                tokio::time::sleep(Duration::from_millis(if swept { 4000 } else { 700 })).await;
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════
// OBFS Raw-TCP Transport (Iran escape path)
// ═══════════════════════════════════════════════════════════════
//
// Connects raw TCP to a directly-reachable foreign IP:port (passed via
// `cdn_edge`), runs the OSSH-style obfs handshake so the wire is uniform
// random from byte 0, then speaks phantom's exact framed protocol over it
// using length-delimited messages. No TLS ClientHello, no SNI, no HTTP —
// nothing for Iran's "RST any foreign TLS handshake" classifier to fire on.

async fn run_obfs_loop(
    config: TransportConfig,
    mut upstream_rx: mpsc::Receiver<UpstreamMsg>,
    tunnel_state: Arc<Mutex<TunnelState>>,
) {
    let mut retry_count = 0u32;
    loop {
        let addr = config.connect_addr();
        set_runtime_state("connecting");
        info!("[PHANTOM] OBFS connecting: addr={}", addr);
        match obfs_session(&config, &mut upstream_rx, &tunnel_state).await {
            Ok(()) => {
                warn!("[PHANTOM] OBFS session ended gracefully, reconnecting in 500ms...");
                retry_count = 0;
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            Err(e) => {
                retry_count += 1;
                let delay = std::cmp::min(2u64.pow(retry_count.min(5)), 30);
                set_runtime_state("reconnecting");
                set_runtime_last_error(e.to_string());
                error!(
                    "[PHANTOM] ❌ OBFS FAILED: {} | retry #{} in {}s | addr={}",
                    e, retry_count, delay, addr
                );
                tokio::time::sleep(Duration::from_secs(delay)).await;
            }
        }
    }
}

async fn obfs_session(
    config: &TransportConfig,
    upstream_rx: &mut mpsc::Receiver<UpstreamMsg>,
    tunnel_state: &Arc<Mutex<TunnelState>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use phantom_proto::obfs::{self, ObfsStream};

    let started_at = Instant::now();
    let addr = config.connect_addr();
    let obfs_key = config
        .obfs_key
        .clone()
        .unwrap_or_else(|| "phantom-obfs".to_string());

    let tcp = connect_transport_tcp(config, &addr, "OBFS").await?;
    let _ = tcp.set_nodelay(true);

    let stream = ObfsStream::connect_client(tcp, obfs_key.as_bytes())
        .await
        .map_err(|e| format!("OBFS handshake failed: {}", e))?;
    let (mut rd, mut wr) = tokio::io::split(stream);
    let key = config.key;

    // ── Auth (one length-delimited JSON message, same shape as WS) ──
    let auth_request = build_transport_auth_request(config, Some("mesh_client"));
    let auth_json = serde_json::to_string(&auth_request)?;
    obfs::write_msg(&mut wr, auth_json.as_bytes())
        .await
        .map_err(|e| format!("OBFS auth send failed: {}", e))?;

    let auth_resp = tokio::time::timeout(Duration::from_secs(10), obfs::read_msg(&mut rd))
        .await
        .map_err(|_| "OBFS auth timeout: server did not respond within 10s")?
        .map_err(|e| format!("OBFS auth read failed: {}", e))?;
    let json: serde_json::Value = serde_json::from_slice(&auth_resp)
        .map_err(|e| format!("OBFS auth response not JSON: {}", e))?;
    if let Some(err) = json.get("error") {
        return Err(format!("❌ OBFS auth REJECTED by server: {}", err).into());
    }
    let token = json["token"].as_str().unwrap_or("unknown");
    info!(
        "[PHANTOM] ✓ OBFS authenticated — session: {}...",
        &token[..token.len().min(16)]
    );
    mesh::set_status("connected");
    mesh::clear_last_error();
    let ping_ms = started_at.elapsed().as_millis().min(u32::MAX as u128) as u32;
    set_runtime_connected(Some(ping_ms));
    info!("[PHANTOM] ✓ OBFS TUNNEL ACTIVE — relay is live");

    // ── Dedicated reader task ──────────────────────────────────────
    // `obfs::read_msg` uses `read_exact`, which is NOT cancellation-safe.
    // Polling it directly inside `select!` means the send branch firing
    // would drop a half-read message; because ObfsStream advances the rx
    // keystream as bytes are consumed, the next read starts from a
    // misaligned keystream position and decodes a garbage length prefix
    // ("incoming message length exceeds MAX_MSG_LEN"), tearing the tunnel.
    //
    // Owning `rd` in its own task and forwarding whole messages over an
    // mpsc channel makes the relay loop cancel-safe: channel `recv()` can
    // be dropped and re-polled with zero data loss.
    let (down_tx, mut down_rx) = mpsc::channel::<Result<Vec<u8>, String>>(256);
    let reader = tokio::spawn(async move {
        loop {
            match obfs::read_msg(&mut rd).await {
                Ok(msg) => {
                    if down_tx.send(Ok(msg)).await.is_err() {
                        break; // relay loop gone
                    }
                }
                Err(e) => {
                    let _ = down_tx
                        .send(Err(format!("OBFS receive error: {}", e)))
                        .await;
                    break;
                }
            }
        }
    });

    // ── Bidirectional relay (single-loop select!, mirrors ws path) ──
    let relay_result: Result<(), Box<dyn std::error::Error + Send + Sync>> = loop {
        tokio::select! {
            msg = upstream_rx.recv() => {
                match msg {
                    Some(msg) => {
                        let mut frames = Vec::new();
                        process_upstream_msg(msg, &mut frames, tunnel_state).await;
                        while let Ok(msg) = upstream_rx.try_recv() {
                            process_upstream_msg(msg, &mut frames, tunnel_state).await;
                        }
                        if !frames.is_empty() {
                            let plaintext = encode_frames(&frames);
                            let encrypted = encrypt(&key, &plaintext);
                            if let Err(e) = obfs::write_msg(&mut wr, &encrypted).await {
                                break Err(format!("OBFS send failed: {}", e).into());
                            }
                        }
                    }
                    None => {
                        error!("[PHANTOM] OBFS upstream channel closed");
                        break Ok(());
                    }
                }
            }
            framed = down_rx.recv() => {
                let data = match framed {
                    Some(Ok(d)) => d,
                    Some(Err(e)) => break Err(e.into()),
                    None => break Err("OBFS reader task ended".into()),
                };
                if data.is_empty() {
                    continue; // server keepalive
                }
                match decrypt(&key, &data) {
                    Ok(plaintext) => match decode_frames(&plaintext) {
                        Ok(frames) => {
                            let app_frames: Vec<_> = frames
                                .into_iter()
                                .filter(|f| f.cmd != Cmd::Relay)
                                .collect();
                            dispatch_downstream(app_frames, tunnel_state).await;
                        }
                        Err(e) => error!("[PHANTOM] OBFS frame decode error: {}", e),
                    },
                    Err(e) => error!("[PHANTOM] OBFS decrypt failed: {}", e),
                }
            }
        }
    };

    reader.abort();
    relay_result
}

async fn connect_transport_tcp(
    config: &TransportConfig,
    addr: &str,
    context: &'static str,
) -> Result<TcpStream, Box<dyn std::error::Error + Send + Sync>> {
    let Some(raw_proxy) = config
        .upstream_proxy
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return tokio::time::timeout(Duration::from_secs(10), TcpStream::connect(addr))
            .await
            .map_err(|_| format!("{} TCP connect timeout to {}", context, addr))?
            .map_err(|e| format!("{} TCP connect to {} failed: {}", context, addr, e).into());
    };

    let proxy = TransportUpstreamProxy::parse(raw_proxy)
        .map_err(|error| format!("{} upstream proxy config error: {}", context, error))?;
    info!(
        "[PHANTOM] {} first-hop: dialing {} via {} upstream {}",
        context,
        addr,
        proxy.label(),
        proxy.connect_addr()
    );

    match proxy {
        TransportUpstreamProxy::Socks5 { host, port, auth } => {
            dial_via_socks5(context, &host, port, auth.as_ref(), addr).await
        }
        TransportUpstreamProxy::Http { host, port, auth } => {
            dial_via_http_proxy(context, &host, port, auth.as_ref(), addr).await
        }
    }
}

async fn connect_transport_upstream(
    context: &'static str,
    host: &str,
    port: u16,
    proxy_kind: &str,
) -> Result<TcpStream, Box<dyn std::error::Error + Send + Sync>> {
    let connect_addr = format!("{}:{}", host, port);
    tokio::time::timeout(Duration::from_secs(20), TcpStream::connect(&connect_addr))
        .await
        .map_err(|_| {
            format!(
                "{} {} upstream connect timed out: {}",
                context, proxy_kind, connect_addr
            )
        })?
        .map_err(|error| {
            format!(
                "{} {} upstream connect failed: {}: {}",
                context, proxy_kind, connect_addr, error
            )
            .into()
        })
}

async fn dial_via_http_proxy(
    context: &'static str,
    proxy_host: &str,
    proxy_port: u16,
    auth: Option<&TransportUpstreamAuth>,
    target_addr: &str,
) -> Result<TcpStream, Box<dyn std::error::Error + Send + Sync>> {
    let mut stream = connect_transport_upstream(context, proxy_host, proxy_port, "HTTP").await?;
    let _ = stream.set_nodelay(true);
    let auth_header = auth
        .map(|auth| format!("Proxy-Authorization: {}\r\n", auth.basic_header_value()))
        .unwrap_or_default();
    let request = format!(
        "CONNECT {target} HTTP/1.1\r\n\
         Host: {target}\r\n\
         User-Agent: PacketObfs/1\r\n\
         {auth_header}\
         Proxy-Connection: Keep-Alive\r\n\
         Connection: Keep-Alive\r\n\
         \r\n",
        target = target_addr,
        auth_header = auth_header,
    );
    stream
        .write_all(request.as_bytes())
        .await
        .map_err(|error| format!("{} HTTP upstream CONNECT write failed: {}", context, error))?;

    let response = read_upstream_http_response_head(context, &mut stream).await?;
    let status_line = response.lines().next().unwrap_or("").trim();
    if !status_line.contains(" 200 ") {
        let carrier_error = extract_http_header(&response, "x-packet-carrier-error")
            .map(|detail| format!("; carrier={}", detail))
            .unwrap_or_default();
        let status = if status_line.is_empty() {
            "closed before HTTP response"
        } else {
            status_line
        };
        return Err(format!(
            "{} HTTP upstream CONNECT failed: {}{}",
            context, status, carrier_error
        )
        .into());
    }

    Ok(stream)
}

async fn dial_via_socks5(
    context: &'static str,
    proxy_host: &str,
    proxy_port: u16,
    auth: Option<&TransportUpstreamAuth>,
    target_addr: &str,
) -> Result<TcpStream, Box<dyn std::error::Error + Send + Sync>> {
    let (target_host, target_port) = split_target_host_port(target_addr)?;
    let mut stream = connect_transport_upstream(context, proxy_host, proxy_port, "SOCKS5").await?;
    let _ = stream.set_nodelay(true);

    if auth.is_some() {
        stream
            .write_all(&[0x05, 0x02, 0x00, 0x02])
            .await
            .map_err(|error| format!("{} SOCKS5 upstream auth write failed: {}", context, error))?;
    } else {
        stream
            .write_all(&[0x05, 0x01, 0x00])
            .await
            .map_err(|error| format!("{} SOCKS5 upstream auth write failed: {}", context, error))?;
    }

    let method_reply = read_exact_upstream(&mut stream, 2, context, "SOCKS5 upstream auth").await?;
    if method_reply.first().copied() != Some(0x05) {
        return Err(format!(
            "{} SOCKS5 upstream sent invalid auth reply: {:02x?}",
            context, method_reply
        )
        .into());
    }

    match method_reply.get(1).copied() {
        Some(0x00) => {}
        Some(0x02) => {
            let Some(auth) = auth else {
                return Err(format!(
                    "{} SOCKS5 upstream requested username/password but none was configured",
                    context
                )
                .into());
            };
            authenticate_upstream_socks5(context, &mut stream, auth).await?;
        }
        Some(0xff) => {
            return Err(format!("{} SOCKS5 upstream rejected all auth methods", context).into())
        }
        Some(method) => {
            return Err(format!(
                "{} SOCKS5 upstream selected unsupported auth method {}",
                context, method
            )
            .into())
        }
        None => return Err(format!("{} SOCKS5 upstream auth reply was truncated", context).into()),
    }

    let host_bytes = target_host.as_bytes();
    if host_bytes.len() > u8::MAX as usize {
        return Err(format!("{} SOCKS5 upstream target host is too long", context).into());
    }
    let mut request = Vec::with_capacity(7 + host_bytes.len());
    request.extend_from_slice(&[0x05, 0x01, 0x00, 0x03, host_bytes.len() as u8]);
    request.extend_from_slice(host_bytes);
    request.extend_from_slice(&target_port.to_be_bytes());
    stream.write_all(&request).await.map_err(|error| {
        format!(
            "{} SOCKS5 upstream CONNECT write failed: {}",
            context, error
        )
    })?;

    let reply = read_exact_upstream(&mut stream, 4, context, "SOCKS5 upstream CONNECT").await?;
    if reply[0] != 0x05 || reply[1] != 0x00 {
        return Err(format!(
            "{} SOCKS5 upstream CONNECT failed: {}",
            context,
            socks5_reply_label(reply.get(1).copied())
        )
        .into());
    }
    consume_upstream_socks5_bind_address(context, &mut stream, reply[3]).await?;
    Ok(stream)
}

async fn authenticate_upstream_socks5(
    context: &'static str,
    stream: &mut TcpStream,
    auth: &TransportUpstreamAuth,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let username = auth.username.as_bytes();
    let password = auth.password.as_bytes();
    if username.len() > u8::MAX as usize || password.len() > u8::MAX as usize {
        return Err(format!("{} SOCKS5 upstream username/password is too long", context).into());
    }

    let mut request = Vec::with_capacity(3 + username.len() + password.len());
    request.push(0x01);
    request.push(username.len() as u8);
    request.extend_from_slice(username);
    request.push(password.len() as u8);
    request.extend_from_slice(password);
    stream.write_all(&request).await.map_err(|error| {
        format!(
            "{} SOCKS5 upstream username/password write failed: {}",
            context, error
        )
    })?;

    let reply =
        read_exact_upstream(stream, 2, context, "SOCKS5 upstream username/password auth").await?;
    if reply.as_slice() != [0x01, 0x00] {
        return Err(format!(
            "{} SOCKS5 upstream username/password rejected: {:02x?}",
            context, reply
        )
        .into());
    }
    Ok(())
}

async fn read_exact_upstream(
    stream: &mut TcpStream,
    len: usize,
    transport_context: &'static str,
    operation: &'static str,
) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    let mut buf = vec![0u8; len];
    tokio::time::timeout(Duration::from_secs(20), stream.read_exact(&mut buf))
        .await
        .map_err(|_| format!("{} {} timed out", transport_context, operation))?
        .map_err(|error| format!("{} {} failed: {}", transport_context, operation, error))?;
    Ok(buf)
}

async fn consume_upstream_socks5_bind_address(
    context: &'static str,
    stream: &mut TcpStream,
    address_type: u8,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    match address_type {
        0x01 => {
            let _ = read_exact_upstream(stream, 6, context, "SOCKS5 upstream bind address").await?;
        }
        0x03 => {
            let len = read_exact_upstream(stream, 1, context, "SOCKS5 upstream bind domain length")
                .await?[0] as usize;
            let _ = read_exact_upstream(stream, len + 2, context, "SOCKS5 upstream bind domain")
                .await?;
        }
        0x04 => {
            let _ = read_exact_upstream(stream, 18, context, "SOCKS5 upstream bind IPv6").await?;
        }
        other => {
            return Err(format!(
                "{} SOCKS5 upstream unsupported bind type {}",
                context, other
            )
            .into())
        }
    }
    Ok(())
}

async fn read_upstream_http_response_head(
    context: &'static str,
    stream: &mut TcpStream,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let mut response = Vec::new();
    let mut buf = [0u8; 512];
    loop {
        let n = tokio::time::timeout(Duration::from_secs(20), stream.read(&mut buf))
            .await
            .map_err(|_| format!("{} HTTP upstream CONNECT response timed out", context))?
            .map_err(|error| {
                format!(
                    "{} HTTP upstream CONNECT response failed: {}",
                    context, error
                )
            })?;
        if n == 0 {
            break;
        }
        response.extend_from_slice(&buf[..n]);
        if response.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
        if response.len() > 8192 {
            return Err(format!(
                "{} HTTP upstream CONNECT response headers are too large",
                context
            )
            .into());
        }
    }
    Ok(String::from_utf8_lossy(&response).into_owned())
}

fn extract_http_header(response: &str, header_name: &str) -> Option<String> {
    response.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        if name.trim().eq_ignore_ascii_case(header_name) {
            Some(value.trim().to_string())
        } else {
            None
        }
    })
}

fn split_target_host_port(target_addr: &str) -> Result<(String, u16), String> {
    if let Some(without_open) = target_addr.strip_prefix('[') {
        let (host, rest) = without_open.split_once(']').ok_or_else(|| {
            format!(
                "target IPv6 address is missing closing bracket: {}",
                target_addr
            )
        })?;
        let port = rest
            .strip_prefix(':')
            .ok_or_else(|| format!("target address is missing port: {}", target_addr))?;
        let port = port
            .parse::<u16>()
            .map_err(|_| format!("target port is invalid: {}", target_addr))?;
        return Ok((host.to_string(), port));
    }

    let (host, port) = target_addr
        .rsplit_once(':')
        .ok_or_else(|| format!("target address is missing port: {}", target_addr))?;
    if host.contains(':') {
        return Err(format!(
            "target IPv6 address must be bracketed before using upstream proxy: {}",
            target_addr
        ));
    }
    let port = port
        .parse::<u16>()
        .map_err(|_| format!("target port is invalid: {}", target_addr))?;
    Ok((host.to_string(), port))
}

fn socks5_reply_label(code: Option<u8>) -> &'static str {
    match code {
        Some(0x01) => "general failure",
        Some(0x02) => "connection not allowed",
        Some(0x03) => "network unreachable",
        Some(0x04) => "host unreachable",
        Some(0x05) => "connection refused",
        Some(0x06) => "TTL expired",
        Some(0x07) => "command not supported",
        Some(0x08) => "address type not supported",
        Some(0x00) => "succeeded",
        _ => "unknown error",
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
        set_runtime_state("connecting");
        info!(
            "[PHANTOM] WS connecting: addr={} host={} cdn_edge={:?} tls={}",
            addr,
            host,
            config.cdn_edge,
            config.is_tls()
        );

        match ws_session(&config, &mut upstream_rx, &tunnel_state).await {
            Ok(()) => {
                warn!("[PHANTOM] WebSocket session ended gracefully, reconnecting in 500ms...");
                retry_count = 0;
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            Err(e) => {
                retry_count += 1;
                let delay = std::cmp::min(2u64.pow(retry_count.min(5)), 30);
                set_runtime_state("reconnecting");
                set_runtime_last_error(e.to_string());
                error!(
                    "[PHANTOM] ❌ WS FAILED: {} | retry #{} in {}s | addr={} host={}",
                    e, retry_count, delay, addr, host
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
    let started_at = Instant::now();
    let host = config.host_value();
    let connect_addr = config.connect_addr();

    // Build the WebSocket upgrade request
    // This is crafted to look like a normal browser WebSocket connection
    let ws_scheme = if config.is_tls() { "wss" } else { "ws" };
    let ws_uri = format!("{}://{}/api/v1/lessons/live", ws_scheme, host);
    let ws_key = tokio_tungstenite::tungstenite::handshake::client::generate_key();

    let origin = if config.is_tls() {
        format!("https://{}", host)
    } else {
        format!("http://{}", host)
    };
    let ws_user_agent = config
        .user_agent_override
        .clone()
        .unwrap_or_else(|| random_user_agent().to_string());

    let request = http::Request::builder()
        .uri(&ws_uri)
        .header("Host", &host)
        .header("Origin", origin)
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header("Sec-WebSocket-Key", &ws_key)
        .header("User-Agent", ws_user_agent)
        .header("Accept-Language", "en-US,en;q=0.9,fa;q=0.8")
        .header("Cache-Control", "no-cache")
        .header("Pragma", "no-cache")
        .body(())?;

    // Establish TCP connection to server (or CDN edge). Packet Chain uses
    // `upstream_proxy` here to reach Packet WebSocket through DirectSock.
    info!("[PHANTOM] TCP connecting to {}...", connect_addr);
    let tcp = connect_transport_tcp(config, &connect_addr, "WS").await?;
    let _ = tcp.set_nodelay(true);
    info!("[PHANTOM] ✓ TCP connected to {}", connect_addr);

    let tcp = if config.fragment_enabled {
        info!(
            "[PHANTOM] TCP fragmentation enabled (v2rayNG tlshello: 5x random 100-150B, 10-20ms delays) for {} handshake",
            if config.is_tls() {
                "TLS"
            } else {
                "plaintext WS"
            }
        );
        FragmentStream::new(tcp, config.fragment_size)
    } else {
        FragmentStream::passthrough(tcp)
    };

    // Apply TLS wrapper if using WSS (HTTPS)
    if config.is_tls() {
        let sni = config.sni_value();
        info!("[PHANTOM] Starting TLS handshake. SNI: {}", sni);
        if !config.spki_pins.is_empty() {
            info!(
                "[PHANTOM] Bridge SPKI pinning enabled ({} pins)",
                normalize_spki_pins(&config.spki_pins).len()
            );
        }
        if config.uses_browser_like_tls() {
            // ── Chrome-fingerprinted path (BoringSSL) ──────────────────
            // Iran RSTs the rustls JA3; a real Chrome ClientHello (what
            // v2ray's fp=chrome sends) is what survives. tcp is already a
            // FragmentStream so the ClientHello is also TCP-fragmented.
            info!("[PHANTOM] TLS engine: BoringSSL (Chrome JA3, fp=chrome equivalent)");
            let alpn: Vec<&[u8]> = vec![b"h2", b"http/1.1"];
            let tls_stream = tokio::time::timeout(
                Duration::from_secs(15),
                crate::chrome_tls::connect_chrome(tcp, &sni, &alpn),
            )
            .await
            .map_err(|_| {
                format!(
                    "Chrome TLS handshake timed out (DPI blackhole? SNI: {})",
                    sni
                )
            })?
            .map_err(|e| format!("{}", e))?;

            info!(
                "[PHANTOM] WS handshake: upgrade request to {} via Chrome TLS",
                ws_uri
            );
            let (ws_stream, response) = tokio::time::timeout(
                Duration::from_secs(15),
                tokio_tungstenite::client_async(request, tls_stream),
            )
            .await
            .map_err(|_| "WebSocket TLS handshake timed out (DPI blackhole or CDN drop)")?
            .map_err(|e| format!("WebSocket handshake failed: {}", e))?;

            info!(
                "[PHANTOM] ✓ WebSocket connected (Chrome TLS) — HTTP {} from {}",
                response.status(),
                host
            );
            ws_auth_and_relay(ws_stream, config, upstream_rx, tunnel_state, started_at).await
        } else {
            let tls_config = build_tls_client_config(config)?;

            // For custom CDNs or tunnels, we might not want to verify the destination IP strictly
            // For production we'd want custom verifiers, but standard works for Cloudflare
            let connector = TlsConnector::from(Arc::new(tls_config));

            let server_name = ServerName::try_from(sni.clone())
                .map_err(|e| format!("Invalid FQDN for SNI ({}): {}", sni, e))?
                .to_owned();

            let tls_stream =
                tokio::time::timeout(Duration::from_secs(15), connector.connect(server_name, tcp))
                    .await
                    .map_err(|_| format!("TLS handshake timed out (DPI blackhole? SNI: {})", sni))?
                    .map_err(|e| format!("TLS handshake failed (SNI: {}): {}", sni, e))?;

            info!(
                "[PHANTOM] WS handshake: upgrade request to {} via TLS",
                ws_uri
            );
            let (ws_stream, response) = tokio::time::timeout(
                Duration::from_secs(15),
                tokio_tungstenite::client_async(request, tls_stream),
            )
            .await
            .map_err(|_| "WebSocket TLS handshake timed out (DPI blackhole or CDN drop)")?
            .map_err(|e| format!("WebSocket handshake failed: {}", e))?;

            info!(
                "[PHANTOM] ✓ WebSocket connected — HTTP {} from {}",
                response.status(),
                host
            );
            ws_auth_and_relay(ws_stream, config, upstream_rx, tunnel_state, started_at).await
        }
    } else {
        // Plaintext WS handshake over the raw TCP stream
        info!(
            "[PHANTOM] WS handshake: upgrade request to {} via plain HTTP transport",
            ws_uri
        );
        let (ws_stream, response) = tokio::time::timeout(
            Duration::from_secs(15),
            tokio_tungstenite::client_async(request, tcp),
        )
        .await
        .map_err(|_| "WebSocket plaintext handshake timed out (DPI blackhole or CDN drop)")?
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
        ws_auth_and_relay(ws_stream, config, upstream_rx, tunnel_state, started_at).await
    }
}

/// Authenticate over WebSocket, then run bidirectional relay.
/// Generic over stream type to support plain TCP, TLS, and fragmented TLS.
async fn ws_auth_and_relay<S>(
    ws_stream: WebSocketStream<S>,
    config: &TransportConfig,
    upstream_rx: &mut mpsc::Receiver<UpstreamMsg>,
    tunnel_state: &Arc<Mutex<TunnelState>>,
    started_at: Instant,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    let (mut ws_sender, mut ws_receiver) = ws_stream.split();
    let key = config.key;

    // ── Authenticate ──
    let auth_request = build_transport_auth_request(config, Some("mesh_client"));
    let auth_json = serde_json::to_string(&auth_request)?;

    info!("[PHANTOM] Sending auth (ts={})...", auth_request.ts);
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
            mesh::set_status("connected");
            mesh::clear_last_error();
            let ping_ms = started_at.elapsed().as_millis().min(u32::MAX as u128) as u32;
            set_runtime_connected(Some(ping_ms));
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

    loop {
        tokio::select! {
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

                        let relay_frames = mesh::drain_relay_frames();
                        for relay in relay_frames {
                            let queue_latency_ms =
                                relay.received_at.elapsed().as_millis().min(u64::MAX as u128)
                                    as u64;
                            mesh::record_relay_success(
                                &relay.next_peer_id,
                                queue_latency_ms,
                            );
                            debug!(
                                "[PHANTOM] Queuing relay frame for peer {} ({}B)",
                                relay.next_peer_id,
                                relay.payload.len()
                            );
                            // Until peer-target metadata is represented as a first-class protocol
                            // frame, ship the peeled onion payload over the relay command itself.
                            frames.push(Frame::relay(0, relay.payload));
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
                                        let mut app_frames = Vec::new();
                                        for frame in frames {
                                            if frame.cmd == Cmd::Relay {
                                                if let Some(inner) = mesh::process_relay_fragment(&frame.data) {
                                                    debug!("[PHANTOM] Received final-hop mesh fragment payload: {}B", inner.len());
                                                }
                                            } else {
                                                app_frames.push(frame);
                                            }
                                        }
                                        dispatch_downstream(app_frames, tunnel_state).await;
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
                    Some(Ok(Message::Ping(_))) | Some(Ok(Message::Pong(_))) => continue,
                    Some(Ok(Message::Close(reason))) => {
                        info!("[PHANTOM] WebSocket closed by server: {:?}", reason);
                        mesh::set_status("disconnected");
                        return Ok(());
                    }
                    Some(Err(e)) => {
                        mesh::set_last_error(e.to_string());
                        return Err(format!("WS receive error: {} (CDN timeout or network drop)", e).into());
                    }
                    None => {
                        mesh::set_status("disconnected");
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
    let mut client_builder = Client::builder()
        .pool_max_idle_per_host(4)
        .timeout(Duration::from_secs(30));

    if config.is_tls() {
        match build_tls_client_config(&config) {
            Ok(tls_config) => {
                if !config.spki_pins.is_empty() {
                    info!(
                        "[PHANTOM] HTTP bridge SPKI pinning enabled ({} pins)",
                        normalize_spki_pins(&config.spki_pins).len()
                    );
                }
                client_builder = client_builder.use_preconfigured_tls(tls_config);
            }
            Err(error) => {
                mesh::set_status("failed");
                mesh::set_last_error(error.clone());
                set_runtime_state("failed");
                set_runtime_last_error(error.clone());
                error!("[PHANTOM] ❌ {}", error);
                return;
            }
        }
    }

    if let Some((request_host, connect_addr)) = config.http_resolve_override() {
        info!(
            "[PHANTOM] HTTP resolve override: {} -> {}",
            request_host, connect_addr
        );
        client_builder = client_builder.resolve(&request_host, connect_addr);
    }

    if config.uses_browser_like_tls() {
        info!("[PHANTOM] Browser-like TLS profile enabled: ALPN=h2,http/1.1 headers=Chrome-like");
    }

    let http_client = client_builder.build().expect("failed to build HTTP client");

    // Authenticate with server
    let auth_url = config.http_request_url("/api/v1/auth/login");
    info!(
        "[PHANTOM] HTTP authenticating: url={} host={} connect_addr={}",
        auth_url,
        config
            .http_host_header()
            .unwrap_or_else(|| "(default)".to_string()),
        config.connect_addr()
    );
    set_runtime_state("authenticating");
    let token = loop {
        match http_authenticate(&http_client, &config).await {
            Ok((t, ping_ms)) => {
                mesh::set_status("connected");
                mesh::clear_last_error();
                set_runtime_connected(Some(ping_ms));
                break t;
            }
            Err(e) => {
                mesh::set_status("degraded");
                mesh::set_last_error(e.clone());
                set_runtime_last_error(e.clone());
                error!(
                    "[PHANTOM] ❌ HTTP auth failed: {} | url={} | host={} | retrying in 5s...",
                    e,
                    auth_url,
                    config
                        .http_host_header()
                        .unwrap_or_else(|| "(default)".to_string())
                );
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        }
    };

    info!("HTTP authenticated: {}...", &token[..16]);

    let sync_url = config.http_request_url("/api/v1/lessons/sync");
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

        let sync_started = Instant::now();
        let mut request = http_client.post(&sync_url);
        if let Some(host) = config.http_host_header() {
            request = request.header(HOST, host);
        }
        request = apply_browser_like_http_headers(request, &config);

        match request.json(&req_body).send().await {
            Ok(resp) => {
                let ping_ms = sync_started.elapsed().as_millis().min(u32::MAX as u128) as u32;
                set_runtime_ping(ping_ms);
                set_runtime_connected(None);
                let status = resp.status();
                let content_type = resp
                    .headers()
                    .get(reqwest::header::CONTENT_TYPE)
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or("unknown")
                    .to_string();
                let body = match resp.text().await {
                    Ok(text) => text,
                    Err(e) => {
                        let message = format!("Sync response read error: {} (HTTP {})", e, status);
                        set_runtime_last_error(message.clone());
                        error!("[PHANTOM] {}", message);
                        continue;
                    }
                };

                match serde_json::from_str::<SyncResponse>(&body) {
                    Ok(sync_resp) => match b64_decode(&sync_resp.d) {
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
                    },
                    Err(parse_error) => {
                        let server_error = serde_json::from_str::<serde_json::Value>(&body)
                            .ok()
                            .and_then(|json| {
                                json.get("error")
                                    .and_then(|value| value.as_str())
                                    .map(str::to_owned)
                            });

                        let message = if let Some(server_error) = server_error {
                            format!(
                                "Sync rejected by server: {} (HTTP {})",
                                server_error, status
                            )
                        } else {
                            let body_preview = body
                                .chars()
                                .take(180)
                                .collect::<String>()
                                .replace('\n', " ");
                            format!(
                                "Sync response parse error: {} (HTTP {}, content-type={}, body='{}')",
                                parse_error, status, content_type, body_preview
                            )
                        };

                        set_runtime_last_error(message.clone());
                        error!("[PHANTOM] {}", message);
                    }
                }
            }
            Err(e) => {
                set_runtime_state("degraded");
                mesh::set_status("degraded");
                let message = describe_reqwest_error(
                    &e,
                    &sync_url,
                    config.http_host_header().as_deref(),
                    &config.connect_addr(),
                );
                mesh::set_last_error(message.clone());
                set_runtime_last_error(message.clone());
                error!("[PHANTOM] ❌ Sync request failed: {}", message);
                tokio::time::sleep(Duration::from_secs(2)).await;
                consecutive_empty = 0;
            }
        }
    }
}

async fn run_stealth_loop(
    mut config: TransportConfig,
    upstream_rx: mpsc::Receiver<UpstreamMsg>,
    tunnel_state: Arc<Mutex<TunnelState>>,
) {
    config.tls_profile = TlsProfile::BrowserLike;
    if !config.is_tls() {
        let message = "Stealth mode requires an https:// server URL";
        mesh::set_status("failed");
        mesh::set_last_error(message.to_string());
        set_runtime_state("failed");
        set_runtime_last_error(message);
        error!("[PHANTOM] ❌ {}", message);
        return;
    }

    info!("[PHANTOM] Stealth mode: HTTPS POST/polling with browser-like TLS profile");
    run_http_loop(config, upstream_rx, tunnel_state).await;
}

/// HTTP authentication — POST to /api/v1/auth/login
async fn http_authenticate(
    client: &Client,
    config: &TransportConfig,
) -> Result<(String, u32), String> {
    let url = config.http_request_url("/api/v1/auth/login");
    let body = build_transport_auth_request(config, Some("mesh_client"));
    let host_header = config.http_host_header();
    let connect_addr = config.connect_addr();

    let started_at = Instant::now();
    let mut request = client.post(&url);
    if let Some(host) = host_header.as_ref() {
        request = request.header(HOST, host);
    }
    request = apply_browser_like_http_headers(request, config);

    let resp = request
        .json(&body)
        .send()
        .await
        .map_err(|e| describe_reqwest_error(&e, &url, host_header.as_deref(), &connect_addr))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body_preview = resp
            .text()
            .await
            .ok()
            .map(|body| {
                body.chars()
                    .take(180)
                    .collect::<String>()
                    .replace('\n', " ")
            })
            .filter(|body| !body.is_empty());
        let mut message = format!(
            "auth failed: status {} | url={} | connect_addr={}",
            status, url, connect_addr
        );
        if let Some(host_header) = host_header.as_deref() {
            message.push_str(&format!(" | host_header={}", host_header));
        }
        if let Some(body_preview) = body_preview {
            message.push_str(&format!(" | body='{}'", body_preview));
        }
        return Err(message);
    }

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("bad response: {}", e))?;

    let ping_ms = started_at.elapsed().as_millis().min(u32::MAX as u128) as u32;

    json["token"]
        .as_str()
        .map(|s| (s.to_string(), ping_ms))
        .ok_or_else(|| "no token in response".to_string())
}

fn build_transport_auth_request(
    config: &TransportConfig,
    default_mode: Option<&str>,
) -> AuthRequest {
    if let Some(ticket) = config
        .auth_ticket
        .as_ref()
        .filter(|ticket| !ticket.trim().is_empty())
    {
        build_ticket_auth_request(
            ticket.clone(),
            default_mode.map(|mode| mode.to_string()),
            None,
        )
    } else {
        build_auth_request(&config.secret)
    }
}

fn apply_browser_like_http_headers(
    request: reqwest::RequestBuilder,
    config: &TransportConfig,
) -> reqwest::RequestBuilder {
    if !config.uses_browser_like_tls() {
        return request;
    }

    let user_agent = config
        .user_agent_override
        .as_deref()
        .unwrap_or("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36");

    request
        .header(reqwest::header::USER_AGENT, user_agent)
        .header(reqwest::header::ACCEPT, "application/json, text/plain, */*")
        .header(reqwest::header::ACCEPT_LANGUAGE, "en-US,en;q=0.9,fa;q=0.8")
        .header(reqwest::header::CACHE_CONTROL, "no-cache")
        .header("Pragma", "no-cache")
        .header(
            "Sec-CH-UA",
            "\"Chromium\";v=\"124\", \"Google Chrome\";v=\"124\", \"Not-A.Brand\";v=\"99\"",
        )
        .header("Sec-CH-UA-Mobile", "?0")
        .header("Sec-CH-UA-Platform", "\"Windows\"")
        .header("Sec-Fetch-Dest", "empty")
        .header("Sec-Fetch-Mode", "cors")
        .header("Sec-Fetch-Site", "same-origin")
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
            info!(
                "[PHANTOM] Auto mode: trying WebSocket (attempt {}/5) to {}",
                ws_failures + 1,
                config.connect_addr()
            );
            set_runtime_state("connecting");
            match ws_session(&config, &mut upstream_rx, &tunnel_state).await {
                Ok(()) => {
                    ws_failures = 0;
                    info!("[PHANTOM] WS session ended, reconnecting...");
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    continue;
                }
                Err(e) => {
                    ws_failures += 1;
                    set_runtime_state("reconnecting");
                    set_runtime_last_error(e.to_string());
                    error!("[PHANTOM] ❌ Auto WS failed #{}/5: {}", ws_failures, e);
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
            }
        } else {
            warn!("[PHANTOM] WebSocket failed 5x, switching to HTTP polling permanently");
            set_runtime_state("http-fallback");
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

fn describe_reqwest_error(
    error: &reqwest::Error,
    url: &str,
    host_header: Option<&str>,
    connect_addr: &str,
) -> String {
    let mut details = vec![
        format!("request failed: {}", error),
        format!("url={}", url),
        format!("connect_addr={}", connect_addr),
    ];

    if let Some(host_header) = host_header {
        details.push(format!("host_header={}", host_header));
    }

    let mut kinds = Vec::new();
    if error.is_timeout() {
        kinds.push("timeout");
    }
    if error.is_connect() {
        kinds.push("connect");
    }
    if error.is_request() {
        kinds.push("request");
    }
    if error.is_body() {
        kinds.push("body");
    }
    if error.is_decode() {
        kinds.push("decode");
    }
    if !kinds.is_empty() {
        details.push(format!("kind={}", kinds.join(",")));
    }

    if let Some(status) = error.status() {
        details.push(format!("status={}", status));
    }

    let mut causes = Vec::new();
    let mut source = error.source();
    while let Some(current) = source {
        causes.push(current.to_string());
        source = current.source();
    }
    if !causes.is_empty() {
        details.push(format!("causes={}", causes.join(" <- ")));
    }

    details.join(" | ")
}

#[cfg(test)]
mod tests {
    use super::{
        normalize_spki_pins, split_target_host_port, TransportUpstreamAuth, TransportUpstreamProxy,
    };

    #[test]
    fn normalize_spki_pins_strips_prefixes_and_deduplicates() {
        let pins = vec![
            "sha256/abc123=".to_string(),
            " abc123= ".to_string(),
            "SHA256/def456=".to_string(),
            "".to_string(),
        ];

        assert_eq!(
            normalize_spki_pins(&pins),
            vec!["abc123=".to_string(), "def456=".to_string()]
        );
    }

    #[test]
    fn transport_upstream_proxy_parses_socks5() {
        assert_eq!(
            TransportUpstreamProxy::parse("socks5://user:pass@127.0.0.1:10808").unwrap(),
            TransportUpstreamProxy::Socks5 {
                host: "127.0.0.1".to_string(),
                port: 10808,
                auth: Some(TransportUpstreamAuth {
                    username: "user".to_string(),
                    password: "pass".to_string(),
                }),
            }
        );
    }

    #[test]
    fn split_target_host_port_accepts_ipv4_and_bracketed_ipv6() {
        assert_eq!(
            split_target_host_port("103.241.67.247:36571").unwrap(),
            ("103.241.67.247".to_string(), 36571)
        );
        assert_eq!(
            split_target_host_port("[2001:db8::1]:443").unwrap(),
            ("2001:db8::1".to_string(), 443)
        );
    }
}
