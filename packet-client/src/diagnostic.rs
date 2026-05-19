// diagnostic.rs — Side-by-side connection probe
//
// Purpose: figure out *why* a Trojan/WS config that works through the
// Psiphon+v2ray stack fails when run directly by our app. Instead of
// guessing at fingerprints, we measure.
//
// The experiment the operator runs:
//   1. Turn Psiphon ON  → run this probe → copy the report (call it A)
//   2. Turn Psiphon OFF → run this probe → copy the report (call it B)
//
// Because Psiphon is an Android VpnService, when it is ON *every* socket on
// the device — including the sockets this probe opens — is tunnelled through
// Psiphon's escape transport. So report A shows "what the world looks like
// once traffic is already out of Iran" and report B shows "what Iran's DPI
// does to the same traffic when it goes direct". The diff between A and B is
// the exact thing Psiphon provides that our app currently does not.
//
// The single most decisive line is EGRESS IP: if it is Iranian, the probe
// went direct; if it is foreign, the probe was tunnelled. Everything else is
// interpreted relative to that.
//
// Each probe step is classified into a precise outcome rather than a boolean,
// because *how* a connection fails tells us which DPI mechanism hit it:
//   * Connected            → reached the peer
//   * ConnRefused          → peer up, port closed (not censorship)
//   * Timeout              → silent drop  (DPI blackhole / SYN filter)
//   * Reset                → RST injected (active DPI reset)
//   * TlsResetMidHandshake → RST *after* ClientHello (SNI-based block)
//   * TlsOtherError        → handshake failed for a non-censorship reason

use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use std::sync::Mutex;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_rustls::TlsConnector;
use url::Url;

use crate::tls_fragment::FragmentStream;

const STEP_TIMEOUT: Duration = Duration::from_secs(8);
const REACH_TIMEOUT: Duration = Duration::from_secs(4);
const LOCAL_PROXY_FALLBACK_PORTS: &[u16] = &[10808, 1080, 8080, 8118, 8888];
const DISCOVERY_BASELINE: &[(&str, &str, u16)] = &[
    ("Cloudflare anycast", "1.1.1.1", 443),
    ("Google DNS anycast", "8.8.8.8", 443),
    ("Quad9 anycast", "9.9.9.9", 443),
    ("OpenDNS Cisco", "208.67.222.222", 443),
    ("AdGuard DNS", "94.140.14.14", 443),
    ("NextDNS anycast", "45.90.28.0", 443),
    ("CleanBrowsing", "185.228.168.9", 443),
    ("ControlD anycast", "76.76.2.0", 443),
    ("Cloudflare WARP", "162.159.200.1", 443),
    ("Google DNS DoT", "8.8.8.8", 853),
    ("Cloudflare DNS DoT", "1.1.1.1", 853),
    ("Quad9 DNS DoT", "9.9.9.9", 853),
];

/// Parsed pieces of a `trojan://` URI that the probe needs.
#[derive(Clone, Debug, Default)]
struct TrojanTarget {
    host_ip: String,
    port: u16,
    /// HTTP Host header / WebSocket host (e.g. www.creationlong.org).
    ws_host: String,
    /// TLS SNI (often == ws_host).
    sni: String,
    /// WebSocket path (e.g. /assignment).
    ws_path: String,
    upstream_proxy: Option<DiagnosticEndpoint>,
}

#[derive(Clone, Debug)]
struct DiagnosticEndpoint {
    label: String,
    host: String,
    port: u16,
}

fn parse_trojan(uri: &str) -> Result<TrojanTarget, String> {
    // trojan://PASS@HOST:PORT?path=/p&host=h&sni=s&type=ws
    let rest = uri
        .strip_prefix("trojan://")
        .ok_or_else(|| "not a trojan:// uri".to_string())?;
    // Strip the URI fragment (#tag) before anything else — it sits at the
    // end of the query and would otherwise contaminate the last param.
    let rest = rest.split('#').next().unwrap_or(rest);
    let (_, after_at) = rest.split_once('@').unwrap_or(("", rest));
    let (authority, query) = after_at.split_once('?').unwrap_or((after_at, ""));
    let (host, port_s) = authority.rsplit_once(':').unwrap_or((authority, "443"));
    let port: u16 = port_s
        .split('/')
        .next()
        .unwrap_or("443")
        .parse()
        .unwrap_or(443);

    let mut t = TrojanTarget {
        host_ip: host.to_string(),
        port,
        ws_host: host.to_string(),
        sni: host.to_string(),
        ws_path: "/".to_string(),
        upstream_proxy: None,
    };

    for pair in query.split('&') {
        let (k, v) = match pair.split_once('=') {
            Some(kv) => kv,
            None => continue,
        };
        let v = urldecode(v);
        match k {
            "host" => t.ws_host = v,
            "sni" => t.sni = v,
            "path" => t.ws_path = v,
            "upstream" | "upstream_proxy" | "proxy" => {
                t.upstream_proxy = parse_proxy_endpoint(&v, "socks5", "configured upstream")
            }
            "upstream_socks" | "upstream_socks5" | "socks_upstream" => {
                t.upstream_proxy = parse_proxy_endpoint(&v, "socks5", "configured SOCKS upstream")
            }
            "upstream_http" | "http_upstream" => {
                t.upstream_proxy = parse_proxy_endpoint(&v, "http", "configured HTTP upstream")
            }
            _ => {}
        }
    }
    if t.sni.is_empty() {
        t.sni = t.ws_host.clone();
    }
    Ok(t)
}

fn parse_proxy_endpoint(
    raw: &str,
    default_scheme: &str,
    label: &str,
) -> Option<DiagnosticEndpoint> {
    let normalized = if raw.contains("://") {
        raw.to_string()
    } else {
        format!("{}://{}", default_scheme, raw)
    };
    let url = Url::parse(&normalized).ok()?;
    let host = url.host_str()?.to_string();
    let port = url.port()?;
    Some(DiagnosticEndpoint {
        label: label.to_string(),
        host,
        port,
    })
}

fn urldecode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(byte) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(byte as char);
                i += 3;
                continue;
            }
        }
        out.push(b[i] as char);
        i += 1;
    }
    out
}

#[derive(Debug)]
enum Outcome {
    Connected,
    ConnRefused,
    Timeout,
    Reset,
    TlsHandshakeOk { cert_subject: String, alpn: String },
    TlsResetMidHandshake,
    TlsOtherError(String),
    HttpStatus(String),
    Info(String),
    Error(String),
}

impl Outcome {
    fn tag(&self) -> &'static str {
        match self {
            Outcome::Connected => "OK/connected",
            Outcome::ConnRefused => "REFUSED (port closed — not censorship)",
            Outcome::Timeout => "TIMEOUT (silent drop — DPI blackhole/SYN filter)",
            Outcome::Reset => "RST (active reset injected)",
            Outcome::TlsHandshakeOk { .. } => "OK/tls-complete",
            Outcome::TlsResetMidHandshake => "RST-AFTER-CLIENTHELLO (SNI-based block)",
            Outcome::TlsOtherError(_) => "TLS-ERROR",
            Outcome::HttpStatus(_) => "HTTP",
            Outcome::Info(_) => "INFO",
            Outcome::Error(_) => "ERROR",
        }
    }
}

fn classify_io(e: &std::io::Error) -> Outcome {
    use std::io::ErrorKind::*;
    match e.kind() {
        ConnectionReset | ConnectionAborted => Outcome::Reset,
        ConnectionRefused => Outcome::ConnRefused,
        TimedOut => Outcome::Timeout,
        _ => Outcome::Error(format!("{} ({:?})", e, e.kind())),
    }
}

async fn resolve_socket_addr(addr: &str) -> Result<SocketAddr, Outcome> {
    if let Ok(sa) = addr.parse::<SocketAddr>() {
        return Ok(sa);
    }

    tokio::net::lookup_host(addr)
        .await
        .ok()
        .and_then(|mut iter| iter.next())
        .ok_or_else(|| Outcome::Error(format!("DNS resolve failed for {}", addr)))
}

/// A permissive verifier that records the leaf cert subject/issuer and
/// accepts everything. We are diagnosing reachability, not establishing a
/// secure channel — we want the handshake to *complete* so we can see what
/// the far side actually presented.
#[derive(Debug)]
struct CaptureVerifier {
    subject: Arc<Mutex<String>>,
}

impl ServerCertVerifier for CaptureVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        // Best-effort: pull the CN/O out of the DER without a full x509
        // parser by scanning for printable runs. Good enough to recognise
        // "Cloudflare Inc" vs "creationlong.org" vs an injected cert.
        let der = end_entity.as_ref();
        let mut found = String::new();
        let mut run = String::new();
        for &byte in der {
            if byte.is_ascii_graphic() || byte == b' ' {
                run.push(byte as char);
            } else {
                if run.len() >= 4
                    && (run.contains('.') || run.contains("Inc") || run.contains("CA"))
                    && found.len() < 200
                {
                    found.push_str(&run);
                    found.push('|');
                }
                run.clear();
            }
        }
        if let Ok(mut g) = self.subject.lock() {
            *g = found.chars().take(180).collect();
        }
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
            SignatureScheme::RSA_PKCS1_SHA256,
            SignatureScheme::RSA_PSS_SHA256,
            SignatureScheme::ECDSA_NISTP256_SHA256,
            SignatureScheme::ED25519,
            SignatureScheme::RSA_PKCS1_SHA384,
            SignatureScheme::ECDSA_NISTP384_SHA384,
            SignatureScheme::RSA_PKCS1_SHA512,
        ]
    }
}

fn capture_tls_config(alpn: &[&str]) -> (Arc<rustls::ClientConfig>, Arc<Mutex<String>>) {
    let subject = Arc::new(Mutex::new(String::new()));
    let verifier = Arc::new(CaptureVerifier {
        subject: subject.clone(),
    });
    let mut cfg = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();
    cfg.alpn_protocols = alpn.iter().map(|s| s.as_bytes().to_vec()).collect();
    (Arc::new(cfg), subject)
}

async fn probe_tcp(addr: &str) -> (Outcome, u128) {
    let start = Instant::now();
    let sa = match resolve_socket_addr(addr).await {
        Ok(sa) => sa,
        Err(outcome) => return (outcome, start.elapsed().as_millis()),
    };
    match timeout(STEP_TIMEOUT, TcpStream::connect(sa)).await {
        Ok(Ok(_)) => (Outcome::Connected, start.elapsed().as_millis()),
        Ok(Err(e)) => (classify_io(&e), start.elapsed().as_millis()),
        Err(_) => (Outcome::Timeout, start.elapsed().as_millis()),
    }
}

async fn probe_tcp_reach(host: &str, port: u16) -> (Outcome, u128) {
    let start = Instant::now();
    let addr = format!("{}:{}", host, port);
    let sa = match resolve_socket_addr(&addr).await {
        Ok(sa) => sa,
        Err(outcome) => return (outcome, start.elapsed().as_millis()),
    };
    match timeout(REACH_TIMEOUT, TcpStream::connect(sa)).await {
        Ok(Ok(_)) => (Outcome::Connected, start.elapsed().as_millis()),
        Ok(Err(e)) => (classify_io(&e), start.elapsed().as_millis()),
        Err(_) => (Outcome::Timeout, start.elapsed().as_millis()),
    }
}

/// TLS handshake against `ip:port`, sending `sni` (or none if `sni` is None
/// and we connect via an IP ServerName). Returns the outcome and elapsed ms.
async fn probe_tls(ip_port: &str, sni: Option<&str>, alpn: &[&str]) -> (Outcome, u128) {
    let start = Instant::now();
    let sa = match resolve_socket_addr(ip_port).await {
        Ok(sa) => sa,
        Err(outcome) => return (outcome, start.elapsed().as_millis()),
    };
    let tcp = match timeout(STEP_TIMEOUT, TcpStream::connect(sa)).await {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => return (classify_io(&e), start.elapsed().as_millis()),
        Err(_) => return (Outcome::Timeout, start.elapsed().as_millis()),
    };
    let _ = tcp.set_nodelay(true);

    let (cfg, subject) = capture_tls_config(alpn);
    let connector = TlsConnector::from(cfg);

    let server_name: ServerName<'static> = match sni {
        Some(name) => match ServerName::try_from(name.to_string()) {
            Ok(n) => n,
            Err(_) => {
                return (
                    Outcome::Error(format!("bad sni {}", name)),
                    start.elapsed().as_millis(),
                )
            }
        },
        None => ServerName::IpAddress(sa.ip().into()),
    };

    match timeout(STEP_TIMEOUT, connector.connect(server_name, tcp)).await {
        Ok(Ok(stream)) => {
            let alpn_neg = stream
                .get_ref()
                .1
                .alpn_protocol()
                .map(|p| String::from_utf8_lossy(p).into_owned())
                .unwrap_or_else(|| "none".into());
            let subj = subject.lock().map(|g| g.clone()).unwrap_or_default();
            (
                Outcome::TlsHandshakeOk {
                    cert_subject: subj,
                    alpn: alpn_neg,
                },
                start.elapsed().as_millis(),
            )
        }
        Ok(Err(e)) => {
            let es = e.to_string().to_lowercase();
            let out = if es.contains("reset")
                || es.contains("closed")
                || es.contains("eof")
                || es.contains("aborted")
            {
                Outcome::TlsResetMidHandshake
            } else {
                Outcome::TlsOtherError(e.to_string())
            };
            (out, start.elapsed().as_millis())
        }
        Err(_) => (Outcome::Timeout, start.elapsed().as_millis()),
    }
}

/// Full WebSocket upgrade attempt: TLS to ip:port with `sni`, then send the
/// `GET path` upgrade request with `Host: ws_host`, read the response line.
async fn probe_ws_upgrade(t: &TrojanTarget) -> (Outcome, u128) {
    let start = Instant::now();
    let target_addr = format!("{}:{}", t.host_ip, t.port);
    let sa = match resolve_socket_addr(&target_addr).await {
        Ok(sa) => sa,
        Err(outcome) => return (outcome, start.elapsed().as_millis()),
    };
    let tcp = match timeout(STEP_TIMEOUT, TcpStream::connect(sa)).await {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => return (classify_io(&e), start.elapsed().as_millis()),
        Err(_) => return (Outcome::Timeout, start.elapsed().as_millis()),
    };
    let _ = tcp.set_nodelay(true);
    let (cfg, _subj) = capture_tls_config(&["http/1.1"]);
    let connector = TlsConnector::from(cfg);
    let name = match ServerName::try_from(t.sni.clone()) {
        Ok(n) => n,
        Err(_) => {
            return (
                Outcome::Error("bad sni".into()),
                start.elapsed().as_millis(),
            )
        }
    };
    let mut tls = match timeout(STEP_TIMEOUT, connector.connect(name, tcp)).await {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => {
            let es = e.to_string().to_lowercase();
            let out = if es.contains("reset") || es.contains("eof") || es.contains("closed") {
                Outcome::TlsResetMidHandshake
            } else {
                Outcome::TlsOtherError(e.to_string())
            };
            return (out, start.elapsed().as_millis());
        }
        Err(_) => return (Outcome::Timeout, start.elapsed().as_millis()),
    };
    let key = "dGhlIHNhbXBsZSBub25jZQ==";
    let req = format!(
        "GET {path} HTTP/1.1\r\n\
         Host: {host}\r\n\
         Upgrade: websocket\r\n\
         Connection: Upgrade\r\n\
         Sec-WebSocket-Key: {key}\r\n\
         Sec-WebSocket-Version: 13\r\n\
         User-Agent: Mozilla/5.0 (Linux; Android 13) Chrome/121.0.0.0\r\n\
         \r\n",
        path = t.ws_path,
        host = t.ws_host,
        key = key,
    );
    if let Err(e) = tls.write_all(req.as_bytes()).await {
        return (classify_io(&e), start.elapsed().as_millis());
    }
    let mut buf = [0u8; 1024];
    match timeout(STEP_TIMEOUT, tls.read(&mut buf)).await {
        Ok(Ok(0)) => (Outcome::TlsResetMidHandshake, start.elapsed().as_millis()),
        Ok(Ok(n)) => {
            let line = String::from_utf8_lossy(&buf[..n]);
            let first = line.lines().next().unwrap_or("").to_string();
            (Outcome::HttpStatus(first), start.elapsed().as_millis())
        }
        Ok(Err(e)) => (classify_io(&e), start.elapsed().as_millis()),
        Err(_) => (Outcome::Timeout, start.elapsed().as_millis()),
    }
}

/// Same as the full WebSocket probe, but send the TLS ClientHello through the
/// same v2rayNG-style fragmented stream used by DirectSock.
async fn probe_ws_upgrade_fragmented(t: &TrojanTarget, fragment_hint: usize) -> (Outcome, u128) {
    let start = Instant::now();
    let target_addr = format!("{}:{}", t.host_ip, t.port);
    let sa = match resolve_socket_addr(&target_addr).await {
        Ok(sa) => sa,
        Err(outcome) => return (outcome, start.elapsed().as_millis()),
    };
    let tcp = match timeout(STEP_TIMEOUT, TcpStream::connect(sa)).await {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => return (classify_io(&e), start.elapsed().as_millis()),
        Err(_) => return (Outcome::Timeout, start.elapsed().as_millis()),
    };
    let _ = tcp.set_nodelay(true);
    let tcp = FragmentStream::new(tcp, fragment_hint);
    let (cfg, _subj) = capture_tls_config(&["http/1.1"]);
    let connector = TlsConnector::from(cfg);
    let name = match ServerName::try_from(t.sni.clone()) {
        Ok(n) => n,
        Err(_) => {
            return (
                Outcome::Error("bad sni".into()),
                start.elapsed().as_millis(),
            )
        }
    };
    let mut tls = match timeout(STEP_TIMEOUT, connector.connect(name, tcp)).await {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => {
            let es = e.to_string().to_lowercase();
            let out = if es.contains("reset") || es.contains("eof") || es.contains("closed") {
                Outcome::TlsResetMidHandshake
            } else {
                Outcome::TlsOtherError(e.to_string())
            };
            return (out, start.elapsed().as_millis());
        }
        Err(_) => return (Outcome::Timeout, start.elapsed().as_millis()),
    };
    let key = "dGhlIHNhbXBsZSBub25jZQ==";
    let req = format!(
        "GET {path} HTTP/1.1\r\n\
         Host: {host}\r\n\
         Upgrade: websocket\r\n\
         Connection: Upgrade\r\n\
         Sec-WebSocket-Key: {key}\r\n\
         Sec-WebSocket-Version: 13\r\n\
         User-Agent: Mozilla/5.0 (Linux; Android 13) Chrome/121.0.0.0\r\n\
         \r\n",
        path = t.ws_path,
        host = t.ws_host,
        key = key,
    );
    if let Err(e) = tls.write_all(req.as_bytes()).await {
        return (classify_io(&e), start.elapsed().as_millis());
    }
    let mut buf = [0u8; 1024];
    match timeout(STEP_TIMEOUT, tls.read(&mut buf)).await {
        Ok(Ok(0)) => (Outcome::TlsResetMidHandshake, start.elapsed().as_millis()),
        Ok(Ok(n)) => {
            let line = String::from_utf8_lossy(&buf[..n]);
            let first = line.lines().next().unwrap_or("").to_string();
            (Outcome::HttpStatus(first), start.elapsed().as_millis())
        }
        Ok(Err(e)) => (classify_io(&e), start.elapsed().as_millis()),
        Err(_) => (Outcome::Timeout, start.elapsed().as_millis()),
    }
}

/// Fetch the device's public egress IP via a few independent echoes. This is
/// the decisive tunnel-vs-direct signal.
async fn probe_egress() -> Vec<(String, String)> {
    let endpoints = [
        (
            "cloudflare",
            "1.1.1.1",
            "GET /cdn-cgi/trace HTTP/1.1\r\nHost: one.one.one.one\r\nConnection: close\r\n\r\n",
        ),
        (
            "ipify",
            "api.ipify.org:80",
            "GET / HTTP/1.1\r\nHost: api.ipify.org\r\nConnection: close\r\n\r\n",
        ),
    ];
    let mut out = Vec::new();

    // Cloudflare trace over TLS (1.1.1.1:443).
    {
        let r = http_get_tls("1.1.1.1:443", "one.one.one.one", "/cdn-cgi/trace").await;
        let ip = r
            .lines()
            .find(|l| l.starts_with("ip="))
            .map(|l| l.trim_start_matches("ip=").to_string())
            .unwrap_or_else(|| "?".into());
        let loc = r
            .lines()
            .find(|l| l.starts_with("loc="))
            .map(|l| l.trim_start_matches("loc=").to_string())
            .unwrap_or_else(|| "?".into());
        out.push(("cloudflare-trace".into(), format!("ip={} loc={}", ip, loc)));
    }
    let _ = endpoints;
    out
}

async fn http_get_tls(ip_port: &str, host: &str, path: &str) -> String {
    let sa: SocketAddr = match ip_port.parse() {
        Ok(s) => s,
        Err(_) => return String::new(),
    };
    let tcp = match timeout(STEP_TIMEOUT, TcpStream::connect(sa)).await {
        Ok(Ok(s)) => s,
        _ => return String::new(),
    };
    let _ = tcp.set_nodelay(true);
    let (cfg, _s) = capture_tls_config(&["http/1.1"]);
    let connector = TlsConnector::from(cfg);
    let name = match ServerName::try_from(host.to_string()) {
        Ok(n) => n,
        Err(_) => return String::new(),
    };
    let mut tls = match timeout(STEP_TIMEOUT, connector.connect(name, tcp)).await {
        Ok(Ok(s)) => s,
        _ => return String::new(),
    };
    let req = format!(
        "GET {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: curl/8\r\nConnection: close\r\n\r\n",
        path, host
    );
    if tls.write_all(req.as_bytes()).await.is_err() {
        return String::new();
    }
    let mut body = Vec::new();
    let mut buf = [0u8; 2048];
    loop {
        match timeout(STEP_TIMEOUT, tls.read(&mut buf)).await {
            Ok(Ok(0)) | Err(_) => break,
            Ok(Ok(n)) => {
                body.extend_from_slice(&buf[..n]);
                if body.len() > 16384 {
                    break;
                }
            }
            Ok(Err(_)) => break,
        }
    }
    String::from_utf8_lossy(&body).into_owned()
}

async fn read_exact_with_timeout(stream: &mut TcpStream, len: usize) -> Result<Vec<u8>, Outcome> {
    let mut buf = vec![0u8; len];
    match timeout(STEP_TIMEOUT, stream.read_exact(&mut buf)).await {
        Ok(Ok(_)) => Ok(buf),
        Ok(Err(e)) => Err(classify_io(&e)),
        Err(_) => Err(Outcome::Timeout),
    }
}

async fn read_some_with_timeout(stream: &mut TcpStream) -> Result<String, Outcome> {
    let mut body = Vec::new();
    let mut buf = [0u8; 2048];
    loop {
        match timeout(STEP_TIMEOUT, stream.read(&mut buf)).await {
            Ok(Ok(0)) | Err(_) => break,
            Ok(Ok(n)) => {
                body.extend_from_slice(&buf[..n]);
                if body.len() > 16384 {
                    break;
                }
            }
            Ok(Err(e)) => return Err(classify_io(&e)),
        }
    }
    Ok(String::from_utf8_lossy(&body).into_owned())
}

fn parse_trace_identity(raw: &str) -> (String, String) {
    let ip = raw
        .lines()
        .find(|l| l.starts_with("ip="))
        .map(|l| l.trim_start_matches("ip=").trim().to_string())
        .unwrap_or_else(|| "?".into());
    let loc = raw
        .lines()
        .find(|l| l.starts_with("loc="))
        .map(|l| l.trim_start_matches("loc=").trim().to_string())
        .unwrap_or_else(|| "?".into());
    (ip, loc)
}

fn status_line(raw: &str) -> String {
    raw.lines().next().unwrap_or("").trim().to_string()
}

async fn probe_egress_via_socks(port: u16) -> (Outcome, u128) {
    let start = Instant::now();
    let mut stream = match timeout(STEP_TIMEOUT, TcpStream::connect(("127.0.0.1", port))).await {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => return (classify_io(&e), start.elapsed().as_millis()),
        Err(_) => return (Outcome::Timeout, start.elapsed().as_millis()),
    };

    let _ = stream.set_nodelay(true);
    if let Err(e) = stream.write_all(&[0x05, 0x01, 0x00]).await {
        return (classify_io(&e), start.elapsed().as_millis());
    }
    let auth = match read_exact_with_timeout(&mut stream, 2).await {
        Ok(v) => v,
        Err(o) => return (o, start.elapsed().as_millis()),
    };
    if auth.as_slice() != [0x05, 0x00] {
        return (
            Outcome::Error(format!("SOCKS auth rejected: {:02x?}", auth)),
            start.elapsed().as_millis(),
        );
    }

    let host = b"cloudflare.com";
    let mut req = Vec::with_capacity(7 + host.len());
    req.extend_from_slice(&[0x05, 0x01, 0x00, 0x03, host.len() as u8]);
    req.extend_from_slice(host);
    req.extend_from_slice(&80u16.to_be_bytes());
    if let Err(e) = stream.write_all(&req).await {
        return (classify_io(&e), start.elapsed().as_millis());
    }

    let head = match read_exact_with_timeout(&mut stream, 4).await {
        Ok(v) => v,
        Err(o) => return (o, start.elapsed().as_millis()),
    };
    if head[0] != 0x05 || head[1] != 0x00 {
        return (
            Outcome::Error(format!(
                "SOCKS CONNECT failed: {}",
                socks_reply_label(head.get(1).copied())
            )),
            start.elapsed().as_millis(),
        );
    }
    let to_skip = match head[3] {
        0x01 => 6,
        0x03 => match read_exact_with_timeout(&mut stream, 1).await {
            Ok(v) => v[0] as usize + 2,
            Err(o) => return (o, start.elapsed().as_millis()),
        },
        0x04 => 18,
        other => {
            return (
                Outcome::Error(format!("unsupported SOCKS bind address type {}", other)),
                start.elapsed().as_millis(),
            )
        }
    };
    if let Err(o) = read_exact_with_timeout(&mut stream, to_skip).await {
        return (o, start.elapsed().as_millis());
    }

    let http = b"GET /cdn-cgi/trace HTTP/1.1\r\nHost: cloudflare.com\r\nUser-Agent: PacketDiagnostic/1\r\nConnection: close\r\n\r\n";
    if let Err(e) = stream.write_all(http).await {
        return (classify_io(&e), start.elapsed().as_millis());
    }

    match read_some_with_timeout(&mut stream).await {
        Ok(raw) => {
            let (ip, loc) = parse_trace_identity(&raw);
            (
                Outcome::Info(format!("{} ip={} loc={}", status_line(&raw), ip, loc)),
                start.elapsed().as_millis(),
            )
        }
        Err(o) => (o, start.elapsed().as_millis()),
    }
}

async fn probe_egress_via_http_proxy(port: u16) -> (Outcome, u128) {
    let start = Instant::now();
    let mut stream = match timeout(STEP_TIMEOUT, TcpStream::connect(("127.0.0.1", port))).await {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => return (classify_io(&e), start.elapsed().as_millis()),
        Err(_) => return (Outcome::Timeout, start.elapsed().as_millis()),
    };

    let _ = stream.set_nodelay(true);
    let http = b"GET http://cloudflare.com/cdn-cgi/trace HTTP/1.1\r\nHost: cloudflare.com\r\nUser-Agent: PacketDiagnostic/1\r\nConnection: close\r\n\r\n";
    if let Err(e) = stream.write_all(http).await {
        return (classify_io(&e), start.elapsed().as_millis());
    }

    match read_some_with_timeout(&mut stream).await {
        Ok(raw) => {
            let (ip, loc) = parse_trace_identity(&raw);
            (
                Outcome::Info(format!("{} ip={} loc={}", status_line(&raw), ip, loc)),
                start.elapsed().as_millis(),
            )
        }
        Err(o) => (o, start.elapsed().as_millis()),
    }
}

fn socks_reply_label(code: Option<u8>) -> &'static str {
    match code {
        Some(0x01) => "general failure",
        Some(0x02) => "connection not allowed",
        Some(0x03) => "network unreachable",
        Some(0x04) => "host unreachable",
        Some(0x05) => "connection refused",
        Some(0x06) => "TTL expired",
        Some(0x07) => "command not supported",
        Some(0x08) => "address type not supported",
        _ => "unknown reply",
    }
}

fn line(report: &mut String, label: &str, o: &Outcome, ms: u128) {
    report.push_str(&format!("  {:<34} {:>6}ms  {}\n", label, ms, o.tag()));
    match o {
        Outcome::TlsHandshakeOk { cert_subject, alpn } => {
            report.push_str(&format!(
                "      cert: {}\n      alpn: {}\n",
                cert_subject, alpn
            ));
        }
        Outcome::TlsOtherError(e) | Outcome::Error(e) => {
            report.push_str(&format!("      detail: {}\n", e));
        }
        Outcome::HttpStatus(s) => {
            report.push_str(&format!("      status-line: {}\n", s));
        }
        Outcome::Info(s) => {
            report.push_str(&format!("      {}\n", s));
        }
        _ => {}
    }
}

fn local_proxy_ports_to_probe() -> Vec<u16> {
    let mut ports = BTreeSet::new();
    if let Some(json) = crate::runtime_stats_json() {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&json) {
            if let Some(port) = value
                .get("listen_port")
                .and_then(|port| port.as_u64())
                .and_then(|port| u16::try_from(port).ok())
            {
                ports.insert(port);
            }
        }
    }

    ports.extend(LOCAL_PROXY_FALLBACK_PORTS.iter().copied());
    ports.into_iter().collect()
}

/// Run the full probe battery against the given trojan URI and return a
/// human-readable, copy-pasteable report.
pub async fn run_diagnostic(trojan_uri: &str) -> String {
    let mut r = String::new();
    r.push_str("════════ PACKET CONNECTION DIAGNOSTIC ════════\n");
    r.push_str("Run this once with Psiphon ON and once with Psiphon OFF, then\n");
    r.push_str("compare. Raw EGRESS tells you whether this diagnostic process is\n");
    r.push_str("already inside a VPN. Packet excludes its own app process from\n");
    r.push_str("Android VpnService to avoid loops, so Packet ON must be judged by\n");
    r.push_str("the LOCAL PROXY EGRESS section, not raw socket egress.\n\n");

    // ── Egress identity (the decisive signal) ──────────────────────
    r.push_str("[1] EGRESS IDENTITY (tunnel vs direct)\n");
    for (name, val) in probe_egress().await {
        r.push_str(&format!("  {:<20} {}\n", name, val));
    }
    r.push_str("  → Iranian loc/IP = traffic went DIRECT.\n");
    r.push_str("  → Foreign  loc/IP = traffic was TUNNELLED (Psiphon path).\n\n");

    r.push_str("[2] LOCAL PROXY EGRESS (Packet/Psiphon localhost path)\n");
    r.push_str("  These probes hit 127.0.0.1 ports directly. If Packet is connected,\n");
    r.push_str("  the active Packet port should return a foreign/non-Iran exit IP here\n");
    r.push_str("  even when raw EGRESS above is direct. Packet Native may auto-select\n");
    r.push_str("  a port; DirectSock commonly uses 10808.\n");
    for port in local_proxy_ports_to_probe() {
        let (o, ms) = probe_egress_via_socks(port).await;
        line(&mut r, &format!("SOCKS5 127.0.0.1:{}", port), &o, ms);
        let (o, ms) = probe_egress_via_http_proxy(port).await;
        line(&mut r, &format!("HTTP   127.0.0.1:{}", port), &o, ms);
    }
    r.push('\n');

    let t = match parse_trojan(trojan_uri) {
        Ok(t) => t,
        Err(e) => {
            r.push_str(&format!("\nFAILED TO PARSE URI: {}\n", e));
            return r;
        }
    };
    r.push_str(&format!(
        "[3] TARGET\n  ip={}  port={}  sni={}  host={}  path={}\n\n",
        t.host_ip, t.port, t.sni, t.ws_host, t.ws_path
    ));

    let ip_port = format!("{}:{}", t.host_ip, t.port);

    // ── Layer-by-layer against the real target ────────────────────
    r.push_str("[4] TARGET REACHABILITY (layer by layer)\n");
    let (o, ms) = probe_tcp(&ip_port).await;
    line(&mut r, "TCP connect", &o, ms);

    let (o, ms) = probe_tls(&ip_port, Some(&t.sni), &["h2", "http/1.1"]).await;
    line(&mut r, "TLS + config SNI", &o, ms);

    let (o, ms) = probe_tls(&ip_port, None, &["h2", "http/1.1"]).await;
    line(&mut r, "TLS + NO SNI", &o, ms);

    let (o, ms) = probe_tls(&ip_port, Some("www.cloudflare.com"), &["h2", "http/1.1"]).await;
    line(&mut r, "TLS + benign SNI (cloudflare)", &o, ms);

    let (o, ms) = probe_ws_upgrade(&t).await;
    line(&mut r, "WebSocket upgrade (full path)", &o, ms);

    let (o, ms) = probe_ws_upgrade_fragmented(&t, 100).await;
    line(&mut r, "WS upgrade + tlshello fragment", &o, ms);
    r.push_str(
        "  Interpretation: TCP-OK + TLS-config-SNI=RST-AFTER-CLIENTHELLO\n\
        \x20 but TLS-NO-SNI=OK  → SNI-based block on this domain.\n\
        \x20 TCP=TIMEOUT → IP-level blackhole. If fragmented WS is OK while\n\
        \x20 normal WS fails, tlshello fragmentation is enough. If both fail\n\
        \x20 direct but local proxy egress works, a Psiphon-like first hop is\n\
        \x20 the missing layer.\n\n",
    );

    // ── Control reachability baseline ─────────────────────────────
    r.push_str("[5] CONTROL BASELINE\n");
    for (label, hostport, sni) in [
        ("google", "www.google.com:443", "www.google.com"),
        ("cloudflare", "www.cloudflare.com:443", "www.cloudflare.com"),
        ("aparat (IR)", "www.aparat.com:443", "www.aparat.com"),
    ] {
        // Resolve then TLS.
        let resolved = tokio::net::lookup_host(hostport)
            .await
            .ok()
            .and_then(|mut i| i.next());
        match resolved {
            Some(sa) => {
                let (o, ms) = probe_tls(&sa.to_string(), Some(sni), &["h2", "http/1.1"]).await;
                line(&mut r, label, &o, ms);
            }
            None => {
                line(
                    &mut r,
                    label,
                    &Outcome::Error("DNS resolve failed".into()),
                    0,
                );
            }
        }
    }
    r.push('\n');

    // ── Cloudflare pool reachability ──────────────────────────────
    r.push_str("[6] CLOUDFLARE POOL PROBE (which ranges reach you)\n");
    for (label, ip) in [
        ("104.16 (premium)", "104.16.0.1:443"),
        ("104.18 (premium)", "104.18.0.1:443"),
        ("104.21 (free)", "104.21.0.1:443"),
        ("172.64 (spectrum)", "172.64.0.1:443"),
        ("172.67 (free)", "172.67.0.1:443"),
    ] {
        let (o, ms) = probe_tcp(ip).await;
        line(&mut r, label, &o, ms);
    }
    r.push('\n');

    // ── First-hop / hosting reachability discovery ────────────────
    r.push_str("[7] FIRST-HOP DISCOVERY (what can be reached directly)\n");
    r.push_str("  OPEN means TCP SYN/ACK came back from this network. If foreign\n");
    r.push_str("  targets timeout but an Iran bridge is OPEN, use that bridge as\n");
    r.push_str("  DirectSock upstream=http://user:pass@IRAN_IP:PORT. Then the bridge\n");
    r.push_str("  must prove it can reach the foreign Trojan endpoint from the VPS.\n");
    let (o, ms) = probe_tcp_reach(&t.host_ip, t.port).await;
    line(
        &mut r,
        &format!("configured target {}:{}", t.host_ip, t.port),
        &o,
        ms,
    );
    if let Some(upstream) = &t.upstream_proxy {
        let (o, ms) = probe_tcp_reach(&upstream.host, upstream.port).await;
        line(
            &mut r,
            &format!("{} {}:{}", upstream.label, upstream.host, upstream.port),
            &o,
            ms,
        );
    } else {
        r.push_str("  configured upstream                skipped  none in URI\n");
    }
    for (label, host, port) in DISCOVERY_BASELINE {
        let (o, ms) = probe_tcp_reach(host, *port).await;
        line(&mut r, label, &o, ms);
    }

    r.push_str("\n════════ END — copy everything above ════════\n");
    r
}
