//! Trojan-TLS carrier used by the layered Iran bypass stack.
//!
//! This is intentionally separate from Packet's native tunnel transport.
//! The working residential pattern the user provided is:
//!
//! local app/VPN proxy -> mixed HTTP/SOCKS proxy 127.0.0.1:10808
//! -> Trojan TLS carrier -> Cloudflare edge/SNI -> Trojan origin.
//!
//! This module owns the local HTTP CONNECT/SOCKS5 proxy and the
//! Trojan TCP/WS carrier hop.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use sha2::{Digest, Sha224};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;
use tokio_rustls::rustls::pki_types::ServerName;
use tokio_rustls::rustls::{ClientConfig as RustlsClientConfig, RootCertStore};
use tokio_rustls::TlsConnector;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;
use tracing::{debug, error, info, warn};
use url::Url;

use crate::tls_fragment::FragmentStream;
use crate::{
    add_runtime_bytes_down, add_runtime_bytes_up, decrement_runtime_active_streams,
    increment_runtime_active_streams, increment_runtime_total_streams, set_runtime_connected,
    set_runtime_last_error, set_runtime_state,
};

/// The carrier TLS stream is type-erased so the same downstream code drives
/// either the rustls engine or the BoringSSL (Chrome-JA3) engine. Which one
/// is used is decided per-connection by the config's `fp=` fingerprint.
pub trait CarrierIo: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send {}
impl<T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send> CarrierIo for T {}
type CarrierTlsStream = Box<dyn CarrierIo>;
type CarrierWsStream = WebSocketStream<CarrierTlsStream>;

#[derive(Clone, Debug)]
pub struct TrojanCarrierConfig {
    pub protocol: CarrierProtocol,
    pub endpoint: TrojanEndpoint,
    pub fragment_tls_hello: bool,
    pub fragment_size_hint: usize,
}

impl TrojanCarrierConfig {
    pub fn from_uri(uri: &str) -> Result<Self, String> {
        let endpoint = TrojanEndpoint::parse(uri)?;
        Ok(Self {
            protocol: endpoint.protocol,
            endpoint,
            fragment_tls_hello: true,
            fragment_size_hint: 100,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CarrierProtocol {
    Trojan,
    Vless,
}

impl CarrierProtocol {
    fn from_scheme(scheme: &str) -> Result<Self, String> {
        match scheme.to_ascii_lowercase().as_str() {
            "trojan" => Ok(Self::Trojan),
            "vless" => Ok(Self::Vless),
            "vmess" => Err("VMess/vmess:// was detected, but VMess AEAD is not bundled in the embedded Packet engine yet. Use vless:// or trojan:// for the local carrier.".to_string()),
            other => Err(format!(
                "DirectSock carrier URI must start with trojan:// or vless://, got {}://",
                other
            )),
        }
    }

    pub(crate) fn label(&self) -> &'static str {
        match self {
            Self::Trojan => "Trojan",
            Self::Vless => "VLESS",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrojanCarrierTransport {
    Tcp,
    WebSocket,
}

impl TrojanCarrierTransport {
    fn from_uri_value(value: Option<String>) -> Result<Self, String> {
        match value
            .unwrap_or_else(|| "tcp".to_string())
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "" | "tcp" | "raw" => Ok(Self::Tcp),
            "ws" | "websocket" => Ok(Self::WebSocket),
            other => Err(format!(
                "DirectSock supports Trojan type=tcp or type=ws, got type={}",
                other
            )),
        }
    }

    fn label(&self) -> &'static str {
        match self {
            Self::Tcp => "tcp",
            Self::WebSocket => "ws",
        }
    }
}

#[derive(Clone, Debug)]
pub struct TrojanEndpoint {
    pub protocol: CarrierProtocol,
    pub password: String,
    pub connect_host: String,
    pub connect_port: u16,
    pub websocket_path: String,
    pub host: String,
    pub sni: String,
    pub transport: TrojanCarrierTransport,
    pub use_tls: bool,
    pub tls_fingerprint: Option<String>,
    pub alpn_protocols: Vec<Vec<u8>>,
    pub upstream_proxy: Option<CarrierUpstreamProxy>,
}

impl TrojanEndpoint {
    pub fn parse(raw: &str) -> Result<Self, String> {
        let raw = raw.trim();
        if raw.starts_with('{') {
            return Err("V2Ray JSON config was detected. Paste the vless:// outbound URI for this embedded carrier; full V2Ray JSON execution is not bundled yet.".to_string());
        }
        let url = Url::parse(raw).map_err(|error| format!("invalid carrier URI: {}", error))?;
        let protocol = CarrierProtocol::from_scheme(url.scheme())?;

        let password = url.username().trim().to_string();
        if password.is_empty() {
            return Err(format!("{} URI is missing the user component", protocol.label()));
        }
        if protocol == CarrierProtocol::Vless {
            parse_uuid_bytes(&password).map_err(|error| format!("invalid VLESS UUID: {}", error))?;
        }

        let connect_host = url
            .host_str()
            .filter(|host| !host.trim().is_empty())
            .ok_or_else(|| format!("{} URI is missing the edge host", protocol.label()))?
            .to_string();
        let connect_port = url.port().unwrap_or(443);

        let mut query_host = None;
        let mut query_sni = None;
        let mut query_path = None;
        let mut security: Option<String> = None;
        let mut carrier_type = None;
        let mut tls_fingerprint = None;
        let mut alpn_protocols = Vec::new();
        let mut upstream_proxy = None;
        for (key, value) in url.query_pairs() {
            match key.trim().to_ascii_lowercase().as_str() {
                "host" => query_host = Some(value.to_string()),
                "sni" => query_sni = Some(value.to_string()),
                "path" => query_path = Some(value.to_string()),
                "security" => security = Some(value.to_string().to_ascii_lowercase()),
                "type" => carrier_type = Some(value.to_string()),
                "fp" | "fingerprint" => {
                    let value = value.trim();
                    if !value.is_empty() {
                        tls_fingerprint = Some(value.to_ascii_lowercase());
                    }
                }
                "alpn" => {
                    alpn_protocols = parse_alpn_protocols(&value);
                }
                "upstream" | "upstream_proxy" | "proxy" => {
                    upstream_proxy = Some(CarrierUpstreamProxy::parse(&value, None)?);
                }
                "upstream_socks" | "upstream_socks5" | "socks_upstream" => {
                    upstream_proxy = Some(CarrierUpstreamProxy::parse(&value, Some("socks5"))?);
                }
                "upstream_http" | "http_upstream" => {
                    upstream_proxy = Some(CarrierUpstreamProxy::parse(&value, Some("http"))?);
                }
                _ => {}
            }
        }

        let transport = TrojanCarrierTransport::from_uri_value(carrier_type)?;
        let security = security.unwrap_or_else(|| "tls".to_string());
        let use_tls = match security.as_str() {
            "tls" => true,
            "none" | "http" | "plaintext" => false,
            "reality" => return Err("VLESS Reality was detected. Packet's embedded carrier supports VLESS TCP/WS over TLS or plaintext, not Reality yet.".to_string()),
            _ => return Err(format!("{} URI must use security=tls or security=none", protocol.label())),
        };

        let host = query_host
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| connect_host.clone());
        let sni = query_sni
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| host.clone());
        let websocket_path = query_path
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "/".to_string());
        let websocket_path = if websocket_path.starts_with('/') {
            websocket_path
        } else {
            format!("/{}", websocket_path)
        };
        if alpn_protocols.is_empty() && is_chrome_fingerprint(tls_fingerprint.as_deref()) {
            alpn_protocols = match transport {
                TrojanCarrierTransport::Tcp => vec![b"h2".to_vec(), b"http/1.1".to_vec()],
                TrojanCarrierTransport::WebSocket => vec![b"http/1.1".to_vec()],
            };
        }
        if transport == TrojanCarrierTransport::WebSocket
            && !alpn_protocols
                .iter()
                .any(|protocol| protocol.as_slice() == b"http/1.1")
        {
            alpn_protocols.push(b"http/1.1".to_vec());
        }

        Ok(Self {
            protocol,
            password,
            connect_host,
            connect_port,
            websocket_path,
            host,
            sni,
            transport,
            use_tls,
            tls_fingerprint,
            alpn_protocols,
            upstream_proxy,
        })
    }

    pub fn connect_addr(&self) -> String {
        format!("{}:{}", self.connect_host, self.connect_port)
    }

    fn ws_uri(&self) -> String {
        let scheme = if self.use_tls { "wss" } else { "ws" };
        format!("{}://{}{}", scheme, self.host, self.websocket_path)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CarrierUpstreamAuth {
    username: String,
    password: String,
}

impl CarrierUpstreamAuth {
    fn from_url(url: &Url) -> Option<Self> {
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
        let raw = format!("{}:{}", self.username, self.password);
        format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode(raw)
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CarrierUpstreamProxy {
    Socks5 {
        host: String,
        port: u16,
        auth: Option<CarrierUpstreamAuth>,
    },
    Http {
        host: String,
        port: u16,
        auth: Option<CarrierUpstreamAuth>,
    },
}

impl CarrierUpstreamProxy {
    fn parse(raw: &str, default_scheme: Option<&str>) -> Result<Self, String> {
        let raw = raw.trim();
        if raw.is_empty() {
            return Err("DirectSock upstream proxy is empty".to_string());
        }

        let normalized = if raw.contains("://") {
            raw.to_string()
        } else {
            format!("{}://{}", default_scheme.unwrap_or("socks5"), raw)
        };
        let url = Url::parse(&normalized)
            .map_err(|error| format!("invalid DirectSock upstream proxy: {}", error))?;
        let host = url
            .host_str()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| "DirectSock upstream proxy is missing a host".to_string())?
            .to_string();
        let port = url
            .port()
            .ok_or_else(|| "DirectSock upstream proxy is missing a port".to_string())?;
        let auth = CarrierUpstreamAuth::from_url(&url);

        match url.scheme().to_ascii_lowercase().as_str() {
            "socks" | "socks5" => Ok(Self::Socks5 { host, port, auth }),
            "http" | "https" => Ok(Self::Http { host, port, auth }),
            other => Err(format!(
                "DirectSock upstream proxy must be socks5://host:port or http://host:port, got {}",
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

enum CarrierRemote {
    WebSocket(CarrierWsStream),
    Tcp(CarrierTlsStream),
}

fn is_chrome_fingerprint(value: Option<&str>) -> bool {
    matches!(
        value,
        Some("chrome") | Some("chromium") | Some("browser") | Some("browser-like")
    )
}

fn parse_alpn_protocols(value: &str) -> Vec<Vec<u8>> {
    value
        .split(',')
        .filter_map(|protocol| {
            let protocol = protocol.trim();
            if protocol.is_empty() {
                return None;
            }
            match protocol.to_ascii_lowercase().as_str() {
                "h2" => Some(b"h2".to_vec()),
                "http/1.1" => Some(b"http/1.1".to_vec()),
                "http/1.0" => Some(b"http/1.0".to_vec()),
                _ => Some(protocol.as_bytes().to_vec()),
            }
        })
        .collect()
}

#[derive(Debug)]
struct ProxyRequest {
    destination: String,
    is_connect: bool,
    initial_payload: Vec<u8>,
}

pub async fn run_carrier_proxy(
    config: TrojanCarrierConfig,
    std_listener: std::net::TcpListener,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    let listener = match TcpListener::from_std(std_listener) {
        Ok(listener) => listener,
        Err(error) => {
            error!("[carrier] failed to adopt listener: {}", error);
            return;
        }
    };
    let local_addr = listener
        .local_addr()
        .map(|addr| addr.to_string())
        .unwrap_or_else(|_| "127.0.0.1".to_string());
    info!(
        "[carrier] DirectSock {} {} TLS mixed HTTP/SOCKS proxy listening on {} -> {} host={} sni={} path={} fp={} alpn={} upstream={}",
        config.protocol.label(),
        config.endpoint.transport.label(),
        local_addr,
        config.endpoint.connect_addr(),
        config.endpoint.host,
        config.endpoint.sni,
        config.endpoint.websocket_path,
        config
            .endpoint
            .tls_fingerprint
            .as_deref()
            .unwrap_or("default"),
        if config.endpoint.alpn_protocols.is_empty() {
            "default".to_string()
        } else {
            config
                .endpoint
                .alpn_protocols
                .iter()
                .map(|protocol| String::from_utf8_lossy(protocol).to_string())
                .collect::<Vec<_>>()
            .join(",")
        },
        config
            .endpoint
            .upstream_proxy
            .as_ref()
            .map(|proxy| format!("{}://{}", proxy.label(), proxy.connect_addr()))
            .unwrap_or_else(|| "direct".to_string())
    );
    crate::decoy::spawn_decoy_loop(crate::decoy::DecoyConfig::default(), shutdown_rx.clone());
    info!("[carrier] DirectSock decoy cover traffic enabled");
    set_runtime_state("listening");

    loop {
        tokio::select! {
            accepted = listener.accept() => {
                match accepted {
                    Ok((stream, peer)) => {
                        let config = config.clone();
                        tokio::spawn(async move {
                            if let Err(error) = handle_proxy_connection(stream, config).await {
                                debug!("[carrier] proxy client {} ended: {}", peer, error);
                            }
                        });
                    }
                    Err(error) => {
                        warn!("[carrier] accept failed: {}", error);
                    }
                }
            }
            changed = shutdown_rx.changed() => {
                if changed.is_err() || *shutdown_rx.borrow() {
                    info!("[carrier] shutdown requested");
                    break;
                }
            }
        }
    }
}

async fn handle_proxy_connection(
    mut local: TcpStream,
    config: TrojanCarrierConfig,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut first_byte = [0u8; 1];
    read_exact_timeout(&mut local, &mut first_byte, "proxy protocol sniff").await?;

    if first_byte[0] == 0x05 {
        return handle_socks5_connection(local, config).await;
    }

    let request = read_http_proxy_request(&mut local, first_byte[0]).await?;
    increment_runtime_total_streams();
    info!(
        "[carrier] HTTP proxy {} {} via DirectSock {}",
        if request.is_connect {
            "CONNECT"
        } else {
            "FORWARD"
        },
        request.destination,
        config.protocol.label()
    );

    let mut remote = match connect_trojan_remote(&config).await {
        Ok(remote) => remote,
        Err(error) => {
            let detail = format!("DirectSock carrier connect failed: {}", error);
            set_runtime_last_error(&detail);
            write_http_proxy_failure(&mut local, &detail).await?;
            return Err(detail.into());
        }
    };
    set_runtime_state("connecting");
    if let Err(error) = send_carrier_connect(&mut remote, &config, &request.destination).await {
        let detail = format!(
            "DirectSock {} CONNECT to {} failed: {}",
            config.protocol.label(),
            request.destination,
            error
        );
        set_runtime_last_error(&detail);
        write_http_proxy_failure(&mut local, &detail).await?;
        return Err(detail.into());
    }
    increment_runtime_active_streams();
    set_runtime_connected(None);

    if request.is_connect {
        local
            .write_all(b"HTTP/1.1 200 Connection Established\r\nProxy-Agent: PacketCarrier\r\n\r\n")
            .await?;
    } else if !request.initial_payload.is_empty() {
        add_runtime_bytes_up(request.initial_payload.len() as u64);
        send_carrier_payload(
            &mut remote,
            request.initial_payload,
            "carrier initial send failed",
        )
        .await?;
    }

    let result = pump_proxy(local, remote, config.protocol).await;
    decrement_runtime_active_streams();
    result
}

async fn write_http_proxy_failure(
    local: &mut TcpStream,
    detail: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let header_detail = sanitize_http_header_value(detail, 220);
    let response = format!(
        "HTTP/1.1 502 DirectSock Failed\r\n\
         Proxy-Agent: PacketCarrier\r\n\
         X-Packet-Carrier-Error: {header_detail}\r\n\
         Content-Length: 0\r\n\
         Connection: close\r\n\
         \r\n"
    );
    local.write_all(response.as_bytes()).await?;
    Ok(())
}

fn sanitize_http_header_value(value: &str, max_len: usize) -> String {
    value
        .chars()
        .filter(|ch| *ch != '\r' && *ch != '\n')
        .take(max_len)
        .collect()
}

async fn read_http_proxy_request(
    local: &mut TcpStream,
    first_byte: u8,
) -> Result<ProxyRequest, Box<dyn std::error::Error + Send + Sync>> {
    let mut buffer = Vec::with_capacity(2048);
    buffer.push(first_byte);
    let mut chunk = [0u8; 1024];
    loop {
        if buffer.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
        let n = tokio::time::timeout(Duration::from_secs(15), local.read(&mut chunk))
            .await
            .map_err(|_| "proxy request read timed out")??;
        if n == 0 {
            return Err("proxy client closed before request".into());
        }
        buffer.extend_from_slice(&chunk[..n]);
        if buffer.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
        if buffer.len() > 64 * 1024 {
            return Err("proxy request headers are too large".into());
        }
    }

    let header_end = buffer
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|idx| idx + 4)
        .ok_or("proxy request missing header terminator")?;
    let header_bytes = &buffer[..header_end];
    let header_text = std::str::from_utf8(header_bytes)
        .map_err(|_| "proxy request headers are not valid UTF-8")?;
    let mut lines = header_text.split("\r\n");
    let request_line = lines
        .next()
        .ok_or("proxy request is missing request line")?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default();
    let target = parts.next().unwrap_or_default();

    if method.eq_ignore_ascii_case("CONNECT") {
        let destination = normalize_host_port(target, 443)?;
        return Ok(ProxyRequest {
            destination,
            is_connect: true,
            initial_payload: Vec::new(),
        });
    }

    let destination = if let Ok(url) = Url::parse(target) {
        let host = url.host_str().ok_or("absolute proxy URL missing host")?;
        let port = url.port_or_known_default().unwrap_or(80);
        format!("{}:{}", host, port)
    } else {
        let host = header_text
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                if name.eq_ignore_ascii_case("host") {
                    Some(value.trim())
                } else {
                    None
                }
            })
            .ok_or("HTTP proxy request is missing Host header")?;
        normalize_host_port(host, 80)?
    };

    Ok(ProxyRequest {
        destination,
        is_connect: false,
        initial_payload: buffer,
    })
}

async fn handle_socks5_connection(
    mut local: TcpStream,
    config: TrojanCarrierConfig,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let destination = match read_socks5_connect_request(&mut local).await {
        Ok(destination) => destination,
        Err(error) => return Err(error),
    };
    info!(
        "[carrier] SOCKS5 CONNECT {} via DirectSock {}",
        destination,
        config.protocol.label()
    );
    increment_runtime_total_streams();

    let mut remote = match connect_trojan_remote(&config).await {
        Ok(remote) => remote,
        Err(error) => {
            let _ = send_socks5_reply(&mut local, 0x05).await;
            set_runtime_last_error(error.to_string());
            return Err(error);
        }
    };
    set_runtime_state("connecting");
    if let Err(error) = send_carrier_connect(&mut remote, &config, &destination).await {
        let _ = send_socks5_reply(&mut local, 0x05).await;
        set_runtime_last_error(error.to_string());
        return Err(error);
    }
    increment_runtime_active_streams();
    set_runtime_connected(None);

    send_socks5_reply(&mut local, 0x00).await?;
    let result = pump_proxy(local, remote, config.protocol).await;
    decrement_runtime_active_streams();
    result
}

async fn read_socks5_connect_request(
    local: &mut TcpStream,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let mut nmethods = [0u8; 1];
    read_exact_timeout(local, &mut nmethods, "SOCKS5 greeting").await?;
    if nmethods[0] == 0 {
        return Err("SOCKS5 client offered no authentication methods".into());
    }

    let mut methods = vec![0u8; nmethods[0] as usize];
    read_exact_timeout(local, &mut methods, "SOCKS5 auth methods").await?;
    if !methods.contains(&0x00) {
        local.write_all(&[0x05, 0xff]).await?;
        return Err("SOCKS5 client did not offer no-auth method".into());
    }
    local.write_all(&[0x05, 0x00]).await?;

    let mut header = [0u8; 4];
    read_exact_timeout(local, &mut header, "SOCKS5 request header").await?;
    if header[0] != 0x05 {
        return Err("SOCKS5 request has invalid version".into());
    }
    if header[1] != 0x01 {
        send_socks5_reply(local, 0x07).await?;
        return Err("SOCKS5 only supports CONNECT requests".into());
    }
    if header[2] != 0x00 {
        return Err("SOCKS5 request has invalid reserved byte".into());
    }

    let host = match header[3] {
        0x01 => {
            let mut octets = [0u8; 4];
            read_exact_timeout(local, &mut octets, "SOCKS5 IPv4 address").await?;
            Ipv4Addr::from(octets).to_string()
        }
        0x03 => {
            let mut len = [0u8; 1];
            read_exact_timeout(local, &mut len, "SOCKS5 domain length").await?;
            if len[0] == 0 {
                return Err("SOCKS5 domain name is empty".into());
            }
            let mut domain = vec![0u8; len[0] as usize];
            read_exact_timeout(local, &mut domain, "SOCKS5 domain").await?;
            String::from_utf8(domain).map_err(|_| "SOCKS5 domain is not valid UTF-8")?
        }
        0x04 => {
            let mut octets = [0u8; 16];
            read_exact_timeout(local, &mut octets, "SOCKS5 IPv6 address").await?;
            format!("[{}]", Ipv6Addr::from(octets))
        }
        _ => {
            send_socks5_reply(local, 0x08).await?;
            return Err("SOCKS5 address type is unsupported".into());
        }
    };

    let mut port_bytes = [0u8; 2];
    read_exact_timeout(local, &mut port_bytes, "SOCKS5 port").await?;
    let port = u16::from_be_bytes(port_bytes);
    if port == 0 {
        return Err("SOCKS5 destination port is invalid".into());
    }

    Ok(format!("{}:{}", host, port))
}

async fn send_socks5_reply(
    local: &mut TcpStream,
    status: u8,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    local
        .write_all(&[0x05, status, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
        .await?;
    Ok(())
}

async fn read_exact_timeout(
    local: &mut TcpStream,
    buffer: &mut [u8],
    context: &'static str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tokio::time::timeout(Duration::from_secs(15), local.read_exact(buffer))
        .await
        .map_err(|_| format!("{} timed out", context))??;
    Ok(())
}

fn normalize_host_port(target: &str, default_port: u16) -> Result<String, String> {
    let trimmed = target.trim();
    if trimmed.is_empty() {
        return Err("empty proxy destination".to_string());
    }

    if trimmed.starts_with('[') {
        return Ok(trimmed.to_string());
    }

    if trimmed
        .rsplit_once(':')
        .is_some_and(|(_, port)| port.parse::<u16>().is_ok())
    {
        Ok(trimmed.to_string())
    } else {
        Ok(format!("{}:{}", trimmed, default_port))
    }
}

/// Well-known Cloudflare anycast IPs we can rotate to as a first hop when
/// the operator-configured IP is selectively RST'd. The Host header in the
/// trojan URI (e.g. `www.creationlong.org`) routes to the right origin via
/// CF's Host-based routing, so any CF edge IP works — Iran often blocks
/// specific CF IPs while leaving others reachable, and rotating finds the
/// one currently allowed.
const CF_FALLBACK_IPS: &[&str] = &[
    "1.1.1.1",         // cloudflare-dns.com (very stable CF anycast)
    "1.0.0.1",         // cloudflare-dns.com secondary
    "104.16.124.96",   // cdnjs.cloudflare.com (well-known stable CF IP)
    "104.17.171.94",   // ajax.cloudflare.com
    "162.159.135.232", // discord-cf anycast
    "172.64.96.1",     // CF Spectrum (different /24 than 172.64.152)
    "188.114.96.7",    // CF EU anycast
];

fn host_is_ip_literal(host: &str) -> bool {
    host.parse::<std::net::IpAddr>().is_ok()
}

async fn connect_trojan_remote(
    config: &TrojanCarrierConfig,
) -> Result<CarrierRemote, Box<dyn std::error::Error + Send + Sync>> {
    let endpoint_already_chrome = is_chrome_fingerprint(config.endpoint.tls_fingerprint.as_deref());

    // Build the TLS mode matrix for the configured first hop. Chrome JA3 is
    // tried first because it is the only ClientHello that survives Iran's
    // 2026 DPI; the rustls "configured" entries only matter on off-DPI
    // networks.
    let mut primary_modes: Vec<(bool, bool, &'static str)> = Vec::new();
    if config.fragment_tls_hello {
        primary_modes.push((true, true, "fragmented Chrome TLS"));
    }
    primary_modes.push((false, true, "unfragmented Chrome TLS"));
    if !endpoint_already_chrome {
        if config.fragment_tls_hello {
            primary_modes.push((true, false, "fragmented configured TLS"));
        }
        primary_modes.push((false, false, "unfragmented configured TLS"));
    }

    // Build the (host, label, modes) plan. Configured IP first with the
    // full mode matrix; then fallback CF IPs with the single best mode
    // (fragmented Chrome) so total attempts stay bounded.
    let configured_host = config.endpoint.connect_host.clone();
    let configured_is_ip = host_is_ip_literal(&configured_host);
    let mut plan: Vec<(String, &'static str, Vec<(bool, bool, &'static str)>)> = Vec::new();
    plan.push((configured_host.clone(), "configured", primary_modes.clone()));
    if configured_is_ip {
        let best_mode: Vec<(bool, bool, &'static str)> =
            vec![(config.fragment_tls_hello, true, "fragmented Chrome TLS")];
        for alt in CF_FALLBACK_IPS {
            if *alt != configured_host {
                plan.push((alt.to_string(), "CF anycast fallback", best_mode.clone()));
            }
        }
    }

    let mut errors = Vec::new();
    for (host, host_label, modes) in plan {
        // Clone the config and rewrite the first-hop IP. The Host header
        // (`endpoint.host`) stays as configured so CF routes to the same
        // origin regardless of which edge IP we land on.
        let mut alt_config = config.clone();
        alt_config.endpoint.connect_host = host.clone();
        info!(
            "[carrier] first hop {}:{} ({})  sni={} host={} fp={}",
            host,
            alt_config.endpoint.connect_port,
            host_label,
            alt_config.endpoint.sni,
            alt_config.endpoint.host,
            alt_config.endpoint.tls_fingerprint.as_deref().unwrap_or("(unset)"),
        );
        for (fragment_tls_hello, force_chrome_tls, label) in modes {
            let started = std::time::Instant::now();
            match connect_trojan_remote_once(&alt_config, fragment_tls_hello, force_chrome_tls).await {
                Ok(remote) => {
                    info!(
                        "[carrier] DirectSock CONNECTED via {} [{}] in {}ms",
                        host,
                        label,
                        started.elapsed().as_millis()
                    );
                    return Ok(remote);
                }
                Err(error) => {
                    let elapsed_ms = started.elapsed().as_millis();
                    let error_text = error.to_string();
                    warn!(
                        "[carrier] FAIL via {} [{}] after {}ms: {}",
                        host, label, elapsed_ms, error_text
                    );
                    errors.push(format!(
                        "{}/{} ({}ms): {}",
                        host, label, elapsed_ms, error_text
                    ));
                }
            }
        }
    }

    // Post-mortem: classify the failure mode so the next debug step is
    // obvious instead of having to read a wall of RST errors. Most concise
    // taxonomy:
    //   * all attempts = TCP timeout  → IP blackhole (network filter)
    //   * all attempts = RST in TLS   → DPI is RSTing the handshake
    //   * mixed                       → partial reachability
    let kind = if errors.iter().all(|e| e.contains("timed out")) {
        "ALL TIMEOUT — first-hop IPs are blackholed on this network. \
         The carrier proxy / IPv6 / a different network is required."
    } else if errors.iter().all(|e| {
        e.contains("reset by peer")
            || e.contains("RST")
            || e.contains("handshake failed")
            || e.contains("eof")
    }) {
        "ALL TLS-RST — the DPI is RST'ing the TLS handshake on every CF IP \
         from this network. Same Chrome ClientHello passes elsewhere, so \
         the block is connection/destination based, not pure JA3. Try a \
         non-CF first hop (your own server's IP, IPv6) or the carrier proxy."
    } else {
        "MIXED — some first hops timed out, others RST'd. Partial filter."
    };
    error!("[carrier] all attempts failed. Diagnosis: {}", kind);

    Err(format!(
        "all DirectSock carrier attempts failed [{}]: {}",
        kind,
        errors.join(" | ")
    )
    .into())
}

async fn connect_trojan_remote_once(
    config: &TrojanCarrierConfig,
    fragment_tls_hello: bool,
    force_chrome_tls: bool,
) -> Result<CarrierRemote, Box<dyn std::error::Error + Send + Sync>> {
    let stream: CarrierTlsStream = if config.endpoint.use_tls {
        connect_trojan_tls(config, fragment_tls_hello, force_chrome_tls).await?
    } else {
        info!("[carrier] skipping TLS (security=none)");
        let tcp = dial_carrier_tcp(&config.endpoint).await?;
        let _ = tcp.set_nodelay(true);
        Box::new(tcp)
    };

    match config.endpoint.transport {
        TrojanCarrierTransport::Tcp => Ok(CarrierRemote::Tcp(stream)),
        TrojanCarrierTransport::WebSocket => {
            let ws = upgrade_trojan_websocket(&config.endpoint, stream).await?;
            Ok(CarrierRemote::WebSocket(ws))
        }
    }
}

async fn connect_trojan_tls(
    config: &TrojanCarrierConfig,
    fragment_tls_hello: bool,
    force_chrome_tls: bool,
) -> Result<CarrierTlsStream, Box<dyn std::error::Error + Send + Sync>> {
    let endpoint = &config.endpoint;
    let tcp = dial_carrier_tcp(endpoint).await?;
    let _ = tcp.set_nodelay(true);

    let tcp = if fragment_tls_hello {
        FragmentStream::new(tcp, config.fragment_size_hint)
    } else {
        FragmentStream::passthrough(tcp)
    };

    // fp=chrome → BoringSSL Chrome-identical ClientHello (what v2ray sends,
    // what survives Iran's JA3 RST). Anything else → rustls as before.
    if force_chrome_tls || is_chrome_fingerprint(endpoint.tls_fingerprint.as_deref()) {
        info!(
            "[carrier] TLS engine: BoringSSL (Chrome JA3) sni={} forced={}",
            endpoint.sni, force_chrome_tls
        );
        let alpn: Vec<&[u8]> = if endpoint.alpn_protocols.is_empty() {
            vec![b"h2", b"http/1.1"]
        } else {
            endpoint
                .alpn_protocols
                .iter()
                .map(|p| p.as_slice())
                .collect()
        };
        let tls = tokio::time::timeout(
            Duration::from_secs(20),
            crate::chrome_tls::connect_chrome(tcp, &endpoint.sni, &alpn),
        )
        .await
        .map_err(|_| {
            format!(
                "carrier Chrome TLS handshake timed out: sni={}",
                endpoint.sni
            )
        })?
        .map_err(|error| format!("carrier Chrome TLS handshake failed: {}", error))?;
        return Ok(Box::new(tls));
    }

    let mut roots = RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let mut tls_config = RustlsClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    if !endpoint.alpn_protocols.is_empty() {
        tls_config.alpn_protocols = endpoint.alpn_protocols.clone();
    }
    let connector = TlsConnector::from(Arc::new(tls_config));
    let server_name = ServerName::try_from(endpoint.sni.clone())
        .map_err(|error| format!("invalid carrier SNI {}: {}", endpoint.sni, error))?
        .to_owned();
    let tls = tokio::time::timeout(Duration::from_secs(20), connector.connect(server_name, tcp))
        .await
        .map_err(|_| format!("carrier TLS handshake timed out: sni={}", endpoint.sni))?
        .map_err(|error| format!("carrier TLS handshake failed: {}", error))?;

    Ok(Box::new(tls))
}

async fn dial_carrier_tcp(
    endpoint: &TrojanEndpoint,
) -> Result<TcpStream, Box<dyn std::error::Error + Send + Sync>> {
    match endpoint.upstream_proxy.as_ref() {
        None => {
            let connect_addr = endpoint.connect_addr();
            tokio::time::timeout(Duration::from_secs(20), TcpStream::connect(&connect_addr))
                .await
                .map_err(|_| format!("carrier TCP connect timed out: {}", connect_addr))?
                .map_err(|error| {
                    format!("carrier TCP connect failed: {}: {}", connect_addr, error).into()
                })
        }
        Some(CarrierUpstreamProxy::Socks5 { host, port, auth }) => {
            dial_carrier_tcp_via_socks5(host, *port, auth.as_ref(), endpoint).await
        }
        Some(CarrierUpstreamProxy::Http { host, port, auth }) => {
            dial_carrier_tcp_via_http_proxy(host, *port, auth.as_ref(), endpoint).await
        }
    }
}

async fn connect_upstream_proxy(
    host: &str,
    port: u16,
    proxy_kind: &str,
) -> Result<TcpStream, Box<dyn std::error::Error + Send + Sync>> {
    let connect_addr = format!("{}:{}", host, port);
    tokio::time::timeout(Duration::from_secs(20), TcpStream::connect(&connect_addr))
        .await
        .map_err(|_| {
            format!(
                "carrier {} upstream connect timed out: {}",
                proxy_kind, connect_addr
            )
        })?
        .map_err(|error| {
            format!(
                "carrier {} upstream connect failed: {}: {}",
                proxy_kind, connect_addr, error
            )
            .into()
        })
}

async fn dial_carrier_tcp_via_socks5(
    proxy_host: &str,
    proxy_port: u16,
    auth: Option<&CarrierUpstreamAuth>,
    endpoint: &TrojanEndpoint,
) -> Result<TcpStream, Box<dyn std::error::Error + Send + Sync>> {
    let mut stream = connect_upstream_proxy(proxy_host, proxy_port, "SOCKS5").await?;
    let _ = stream.set_nodelay(true);

    if auth.is_some() {
        stream
            .write_all(&[0x05, 0x02, 0x00, 0x02])
            .await
            .map_err(|error| format!("carrier SOCKS5 upstream auth write failed: {}", error))?;
    } else {
        stream
            .write_all(&[0x05, 0x01, 0x00])
            .await
            .map_err(|error| format!("carrier SOCKS5 upstream auth write failed: {}", error))?;
    }
    let method_reply = read_exact_carrier(&mut stream, 2, "carrier SOCKS5 upstream auth").await?;
    if method_reply.first().copied() != Some(0x05) {
        return Err(format!(
            "carrier SOCKS5 upstream sent invalid auth reply: {:02x?}",
            method_reply
        )
        .into());
    }
    match method_reply.get(1).copied() {
        Some(0x00) => {}
        Some(0x02) => {
            let Some(auth) = auth else {
                return Err(
                    "carrier SOCKS5 upstream requested username/password but none was configured"
                        .into(),
                );
            };
            authenticate_socks5_username_password(&mut stream, auth).await?;
        }
        Some(0xff) => return Err("carrier SOCKS5 upstream rejected all auth methods".into()),
        Some(method) => {
            return Err(format!(
                "carrier SOCKS5 upstream selected unsupported auth method {}",
                method
            )
            .into())
        }
        None => return Err("carrier SOCKS5 upstream auth reply was truncated".into()),
    }

    let host_bytes = endpoint.connect_host.as_bytes();
    if host_bytes.len() > u8::MAX as usize {
        return Err("carrier SOCKS5 upstream target host is too long".into());
    }
    let mut request = Vec::with_capacity(7 + host_bytes.len());
    request.extend_from_slice(&[0x05, 0x01, 0x00, 0x03, host_bytes.len() as u8]);
    request.extend_from_slice(host_bytes);
    request.extend_from_slice(&endpoint.connect_port.to_be_bytes());
    stream
        .write_all(&request)
        .await
        .map_err(|error| format!("carrier SOCKS5 upstream CONNECT write failed: {}", error))?;

    let reply = read_exact_carrier(&mut stream, 4, "carrier SOCKS5 upstream CONNECT").await?;
    if reply[0] != 0x05 || reply[1] != 0x00 {
        return Err(format!(
            "carrier SOCKS5 upstream CONNECT failed: {}",
            socks5_reply_label(reply.get(1).copied())
        )
        .into());
    }
    consume_socks5_bind_address(&mut stream, reply[3]).await?;
    Ok(stream)
}

async fn authenticate_socks5_username_password(
    stream: &mut TcpStream,
    auth: &CarrierUpstreamAuth,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let username = auth.username.as_bytes();
    let password = auth.password.as_bytes();
    if username.len() > u8::MAX as usize || password.len() > u8::MAX as usize {
        return Err("carrier SOCKS5 upstream username/password is too long".into());
    }

    let mut request = Vec::with_capacity(3 + username.len() + password.len());
    request.push(0x01);
    request.push(username.len() as u8);
    request.extend_from_slice(username);
    request.push(password.len() as u8);
    request.extend_from_slice(password);
    stream.write_all(&request).await.map_err(|error| {
        format!(
            "carrier SOCKS5 upstream username/password write failed: {}",
            error
        )
    })?;

    let reply =
        read_exact_carrier(stream, 2, "carrier SOCKS5 upstream username/password auth").await?;
    if reply.as_slice() != [0x01, 0x00] {
        return Err(format!(
            "carrier SOCKS5 upstream username/password rejected: {:02x?}",
            reply
        )
        .into());
    }
    Ok(())
}

async fn dial_carrier_tcp_via_http_proxy(
    proxy_host: &str,
    proxy_port: u16,
    auth: Option<&CarrierUpstreamAuth>,
    endpoint: &TrojanEndpoint,
) -> Result<TcpStream, Box<dyn std::error::Error + Send + Sync>> {
    let mut stream = connect_upstream_proxy(proxy_host, proxy_port, "HTTP").await?;
    let _ = stream.set_nodelay(true);
    let target = endpoint.connect_addr();
    let auth_header = auth
        .map(|auth| format!("Proxy-Authorization: {}\r\n", auth.basic_header_value()))
        .unwrap_or_default();
    let request = format!(
        "CONNECT {target} HTTP/1.1\r\n\
         Host: {target}\r\n\
         User-Agent: PacketDirectSock/1\r\n\
         {auth_header}\
         Proxy-Connection: Keep-Alive\r\n\
         Connection: Keep-Alive\r\n\
         \r\n",
        target = target,
        auth_header = auth_header,
    );
    stream
        .write_all(request.as_bytes())
        .await
        .map_err(|error| format!("carrier HTTP upstream CONNECT write failed: {}", error))?;

    let response = read_http_proxy_response_head(&mut stream).await?;
    let status_line = response.lines().next().unwrap_or("");
    if !status_line.contains(" 200 ") {
        return Err(format!("carrier HTTP upstream CONNECT failed: {}", status_line).into());
    }
    Ok(stream)
}

async fn read_exact_carrier(
    stream: &mut TcpStream,
    len: usize,
    context: &'static str,
) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    let mut buf = vec![0u8; len];
    tokio::time::timeout(Duration::from_secs(20), stream.read_exact(&mut buf))
        .await
        .map_err(|_| format!("{} timed out", context))?
        .map_err(|error| format!("{} failed: {}", context, error))?;
    Ok(buf)
}

async fn consume_socks5_bind_address(
    stream: &mut TcpStream,
    address_type: u8,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    match address_type {
        0x01 => {
            let _ = read_exact_carrier(stream, 6, "carrier SOCKS5 upstream bind address").await?;
        }
        0x03 => {
            let len = read_exact_carrier(stream, 1, "carrier SOCKS5 upstream bind domain length")
                .await?[0] as usize;
            let _ =
                read_exact_carrier(stream, len + 2, "carrier SOCKS5 upstream bind domain").await?;
        }
        0x04 => {
            let _ = read_exact_carrier(stream, 18, "carrier SOCKS5 upstream bind IPv6").await?;
        }
        other => {
            return Err(format!("carrier SOCKS5 upstream unsupported bind type {}", other).into())
        }
    }
    Ok(())
}

async fn read_http_proxy_response_head(
    stream: &mut TcpStream,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let mut response = Vec::new();
    let mut buf = [0u8; 512];
    loop {
        let n = tokio::time::timeout(Duration::from_secs(20), stream.read(&mut buf))
            .await
            .map_err(|_| "carrier HTTP upstream CONNECT response timed out")?
            .map_err(|error| format!("carrier HTTP upstream CONNECT response failed: {}", error))?;
        if n == 0 {
            break;
        }
        response.extend_from_slice(&buf[..n]);
        if response.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
        if response.len() > 8192 {
            return Err("carrier HTTP upstream CONNECT response headers are too large".into());
        }
    }
    Ok(String::from_utf8_lossy(&response).into_owned())
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
        _ => "unknown reply",
    }
}

async fn upgrade_trojan_websocket(
    endpoint: &TrojanEndpoint,
    tls: CarrierTlsStream,
) -> Result<CarrierWsStream, Box<dyn std::error::Error + Send + Sync>> {
    let ws_key = tokio_tungstenite::tungstenite::handshake::client::generate_key();
    let request = http::Request::builder()
        .uri(endpoint.ws_uri())
        .header("Host", &endpoint.host)
        .header("Origin", format!("https://{}", endpoint.host))
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header("Sec-WebSocket-Key", ws_key)
        .header("User-Agent", "Mozilla/5.0 (Linux; Android 14; Pixel 7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Mobile Safari/537.36")
        .header("Accept-Language", "fa,en-US;q=0.9,en;q=0.8")
        .header("Cache-Control", "no-cache")
        .header("Pragma", "no-cache")
        .body(())?;

    let (ws, response) = tokio::time::timeout(
        Duration::from_secs(20),
        tokio_tungstenite::client_async(request, tls),
    )
    .await
    .map_err(|_| "carrier WebSocket upgrade timed out")?
    .map_err(|error| format!("carrier WebSocket upgrade failed: {}", error))?;

    if response.status() != http::StatusCode::SWITCHING_PROTOCOLS {
        return Err(format!("carrier WebSocket upgrade returned {}", response.status()).into());
    }

    Ok(ws)
}

async fn send_trojan_connect(
    remote: &mut CarrierRemote,
    password: &str,
    destination: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut payload = Vec::new();
    payload.extend_from_slice(trojan_password_hash(password).as_bytes());
    payload.extend_from_slice(b"\r\n");
    payload.push(0x01); // CONNECT
    payload.extend_from_slice(&encode_trojan_address(destination)?);
    payload.extend_from_slice(b"\r\n");

    send_carrier_payload(remote, payload, "carrier Trojan CONNECT send failed").await?;

    Ok(())
}

async fn send_carrier_connect(
    remote: &mut CarrierRemote,
    config: &TrojanCarrierConfig,
    destination: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    match config.protocol {
        CarrierProtocol::Trojan => {
            send_trojan_connect(remote, &config.endpoint.password, destination).await
        }
        CarrierProtocol::Vless => {
            send_vless_connect(remote, &config.endpoint.password, destination).await
        }
    }
}

async fn send_vless_connect(
    remote: &mut CarrierRemote,
    uuid: &str,
    destination: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut payload = Vec::new();
    payload.push(0x00); // VLESS version
    payload.extend_from_slice(&parse_uuid_bytes(uuid)?);
    payload.push(0x00); // addon length
    payload.push(0x01); // TCP
    payload.extend_from_slice(&encode_vless_address(destination)?);

    send_carrier_payload(remote, payload, "carrier VLESS CONNECT send failed").await?;
    Ok(())
}

async fn send_carrier_payload(
    remote: &mut CarrierRemote,
    payload: Vec<u8>,
    context: &'static str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    match remote {
        CarrierRemote::WebSocket(ws) => {
            ws.send(Message::Binary(payload))
                .await
                .map_err(|error| format!("{}: {}", context, error))?;
        }
        CarrierRemote::Tcp(tls) => {
            tls.write_all(&payload)
                .await
                .map_err(|error| format!("{}: {}", context, error))?;
        }
    }

    Ok(())
}

fn parse_uuid_bytes(uuid: &str) -> Result<[u8; 16], String> {
    let hex: String = uuid.chars().filter(|ch| *ch != '-').collect();
    if hex.len() != 32 || !hex.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err("expected UUID format xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx".to_string());
    }
    let mut out = [0u8; 16];
    for index in 0..16 {
        out[index] = u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16)
            .map_err(|_| "UUID contains invalid hex".to_string())?;
    }
    Ok(out)
}

fn trojan_password_hash(password: &str) -> String {
    let digest = Sha224::digest(password.as_bytes());
    to_lower_hex(&digest)
}

fn to_lower_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn encode_vless_address(destination: &str) -> Result<Vec<u8>, String> {
    let (host, port) = split_destination(destination)?;
    let mut out = Vec::new();
    out.extend_from_slice(&port.to_be_bytes());

    if let Ok(ip) = host.parse::<IpAddr>() {
        match ip {
            IpAddr::V4(ipv4) => {
                out.push(0x01);
                out.extend_from_slice(&ipv4.octets());
            }
            IpAddr::V6(ipv6) => {
                out.push(0x03);
                out.extend_from_slice(&ipv6.octets());
            }
        }
    } else {
        let host_bytes = host.as_bytes();
        if host_bytes.len() > u8::MAX as usize {
            return Err("destination hostname is too long for VLESS address".to_string());
        }
        out.push(0x02);
        out.push(host_bytes.len() as u8);
        out.extend_from_slice(host_bytes);
    }

    Ok(out)
}

fn encode_trojan_address(destination: &str) -> Result<Vec<u8>, String> {
    let (host, port) = split_destination(destination)?;
    let mut out = Vec::new();

    if let Ok(ip) = host.parse::<IpAddr>() {
        match ip {
            IpAddr::V4(ipv4) => {
                out.push(0x01);
                out.extend_from_slice(&ipv4.octets());
            }
            IpAddr::V6(ipv6) => {
                out.push(0x04);
                out.extend_from_slice(&ipv6.octets());
            }
        }
    } else {
        let host_bytes = host.as_bytes();
        if host_bytes.len() > u8::MAX as usize {
            return Err("destination hostname is too long for Trojan address".to_string());
        }
        out.push(0x03);
        out.push(host_bytes.len() as u8);
        out.extend_from_slice(host_bytes);
    }

    out.extend_from_slice(&port.to_be_bytes());
    Ok(out)
}

fn split_destination(destination: &str) -> Result<(String, u16), String> {
    let destination = destination.trim();
    if let Ok(socket) = destination.parse::<SocketAddr>() {
        return Ok((socket.ip().to_string(), socket.port()));
    }

    if let Some(stripped) = destination.strip_prefix('[') {
        let (host, rest) = stripped
            .split_once(']')
            .ok_or_else(|| "invalid bracketed IPv6 destination".to_string())?;
        let port = rest
            .strip_prefix(':')
            .ok_or_else(|| "destination is missing port".to_string())?
            .parse::<u16>()
            .map_err(|_| "destination port is invalid".to_string())?;
        return Ok((host.to_string(), port));
    }

    let (host, port) = destination
        .rsplit_once(':')
        .ok_or_else(|| "destination is missing port".to_string())?;
    if host.is_empty() {
        return Err("destination host is empty".to_string());
    }
    let port = port
        .parse::<u16>()
        .map_err(|_| "destination port is invalid".to_string())?;
    Ok((host.to_string(), port))
}

fn strip_vless_response_header(mut bytes: Vec<u8>) -> Result<Vec<u8>, std::io::Error> {
    if bytes.len() < 2 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "VLESS response header is truncated",
        ));
    }
    let addon_len = bytes[1] as usize;
    let header_len = 2 + addon_len;
    if bytes.len() < header_len {
        return Err(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "VLESS response addon header is truncated",
        ));
    }
    bytes.drain(..header_len);
    Ok(bytes)
}

async fn consume_vless_response_header<R>(reader: &mut R) -> Result<(), std::io::Error>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut header = [0u8; 2];
    reader.read_exact(&mut header).await?;
    let addon_len = header[1] as usize;
    if addon_len > 0 {
        let mut addon = vec![0u8; addon_len];
        reader.read_exact(&mut addon).await?;
    }
    Ok(())
}

async fn pump_proxy(
    local: TcpStream,
    remote: CarrierRemote,
    protocol: CarrierProtocol,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    match remote {
        CarrierRemote::WebSocket(ws) => pump_ws_proxy(local, ws, protocol).await,
        CarrierRemote::Tcp(tls) => pump_tcp_proxy(local, tls, protocol).await,
    }
}

async fn pump_ws_proxy(
    mut local: TcpStream,
    remote: CarrierWsStream,
    protocol: CarrierProtocol,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let (mut ws_sender, mut ws_receiver) = remote.split();
    let (mut local_read, mut local_write) = local.split();
    let local_to_remote = async {
        let mut buffer = [0u8; 16 * 1024];
        loop {
            let n = local_read.read(&mut buffer).await?;
            if n == 0 {
                let _ = ws_sender.send(Message::Close(None)).await;
                break;
            }
            add_runtime_bytes_up(n as u64);
            ws_sender
                .send(Message::Binary(buffer[..n].to_vec()))
                .await
                .map_err(|error| std::io::Error::new(std::io::ErrorKind::BrokenPipe, error))?;
        }
        Ok::<(), std::io::Error>(())
    };

    let remote_to_local = async {
        let mut vless_response_header_pending = protocol == CarrierProtocol::Vless;
        while let Some(message) = ws_receiver.next().await {
            match message
                .map_err(|error| std::io::Error::new(std::io::ErrorKind::BrokenPipe, error))?
            {
                Message::Binary(mut bytes) => {
                    if vless_response_header_pending {
                        vless_response_header_pending = false;
                        bytes = strip_vless_response_header(bytes)?;
                    }
                    add_runtime_bytes_down(bytes.len() as u64);
                    local_write.write_all(&bytes).await?;
                }
                Message::Text(text) => {
                    let mut bytes = text.into_bytes();
                    if vless_response_header_pending {
                        vless_response_header_pending = false;
                        bytes = strip_vless_response_header(bytes)?;
                    }
                    add_runtime_bytes_down(bytes.len() as u64);
                    local_write.write_all(&bytes).await?;
                }
                Message::Ping(_) | Message::Pong(_) => {}
                Message::Close(_) => break,
                Message::Frame(_) => {}
            }
        }
        Ok::<(), std::io::Error>(())
    };

    tokio::select! {
        result = local_to_remote => result?,
        result = remote_to_local => result?,
    }

    Ok(())
}

async fn pump_tcp_proxy(
    mut local: TcpStream,
    remote: CarrierTlsStream,
    protocol: CarrierProtocol,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let (mut remote_read, mut remote_write) = tokio::io::split(remote);
    let (mut local_read, mut local_write) = local.split();

    let local_to_remote = async {
        let mut buffer = [0u8; 16 * 1024];
        loop {
            let n = local_read.read(&mut buffer).await?;
            if n == 0 {
                let _ = remote_write.shutdown().await;
                break;
            }
            add_runtime_bytes_up(n as u64);
            remote_write.write_all(&buffer[..n]).await?;
        }
        Ok::<(), std::io::Error>(())
    };

    let remote_to_local = async {
        if protocol == CarrierProtocol::Vless {
            consume_vless_response_header(&mut remote_read).await?;
        }
        let mut buffer = [0u8; 16 * 1024];
        loop {
            let n = remote_read.read(&mut buffer).await?;
            if n == 0 {
                let _ = local_write.shutdown().await;
                break;
            }
            add_runtime_bytes_down(n as u64);
            local_write.write_all(&buffer[..n]).await?;
        }
        Ok::<(), std::io::Error>(())
    };

    tokio::select! {
        result = local_to_remote => result?,
        result = remote_to_local => result?,
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        encode_trojan_address, read_socks5_connect_request, trojan_password_hash,
        CarrierProtocol, CarrierUpstreamAuth, CarrierUpstreamProxy, TrojanCarrierTransport,
        TrojanEndpoint,
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};

    #[test]
    fn parses_supplied_trojan_ws_uri() {
        let endpoint = TrojanEndpoint::parse(
            "trojan://humanity@172.64.152.23:443?path=%2Fassignment&security=tls&host=www.creationlong.org&type=ws&sni=www.creationlong.org#%40InfoTech_VK",
        )
        .unwrap();

        assert_eq!(endpoint.password, "humanity");
        assert_eq!(endpoint.connect_host, "172.64.152.23");
        assert_eq!(endpoint.connect_port, 443);
        assert_eq!(endpoint.websocket_path, "/assignment");
        assert_eq!(endpoint.host, "www.creationlong.org");
        assert_eq!(endpoint.sni, "www.creationlong.org");
        assert_eq!(endpoint.transport, TrojanCarrierTransport::WebSocket);
        assert_eq!(endpoint.alpn_protocols, vec![b"http/1.1".to_vec()]);
    }

    #[test]
    fn parses_user_trojan_tcp_uri_with_chrome_fingerprint() {
        let endpoint = TrojanEndpoint::parse(
            "trojan://secret@example.com:443?security=tls&headerType=none&fp=chrome&type=tcp&sni=front.example.com&alpn=h2%2Chttp/1.1#user",
        )
        .unwrap();

        assert_eq!(endpoint.password, "secret");
        assert_eq!(endpoint.connect_host, "example.com");
        assert_eq!(endpoint.connect_port, 443);
        assert_eq!(endpoint.host, "example.com");
        assert_eq!(endpoint.sni, "front.example.com");
        assert_eq!(endpoint.transport, TrojanCarrierTransport::Tcp);
        assert_eq!(endpoint.tls_fingerprint.as_deref(), Some("chrome"));
        assert_eq!(
            endpoint.alpn_protocols,
            vec![b"h2".to_vec(), b"http/1.1".to_vec()]
        );
    }

    #[test]
    fn parses_vless_tcp_plain_uri() {
        let endpoint = TrojanEndpoint::parse(
            "vless://86d31821-a522-42ca-b92c-19f50f6fbafe@example.com:443?type=tcp&encryption=none&security=none#plain",
        )
        .unwrap();

        assert_eq!(endpoint.protocol, CarrierProtocol::Vless);
        assert_eq!(endpoint.password, "86d31821-a522-42ca-b92c-19f50f6fbafe");
        assert_eq!(endpoint.connect_host, "example.com");
        assert_eq!(endpoint.connect_port, 443);
        assert_eq!(endpoint.transport, TrojanCarrierTransport::Tcp);
        assert!(!endpoint.use_tls);
    }

    #[test]
    fn routes_vless_reality_out_of_embedded_carrier() {
        let error = TrojanEndpoint::parse(
            "vless://86d31821-a522-42ca-b92c-19f50f6fbafe@35.254.76.153:443?type=tcp&encryption=none&security=reality&pbk=A_m5zPEmi1avKznvYQ5DwI8_Lc8EknEjoTP4yYVwQk8&fp=chrome&sni=www.apple.com&sid=675d633f&spx=%2F#BridgeExit-wa3axgs5",
        )
        .unwrap_err();

        assert!(error.contains("VLESS Reality"));
    }

    #[test]
    fn parses_directsock_upstream_proxy() {
        let endpoint = TrojanEndpoint::parse(
            "trojan://secret@example.com:443?security=tls&type=ws&upstream=socks5%3A%2F%2F127.0.0.1%3A1080",
        )
        .unwrap();

        assert_eq!(
            endpoint.upstream_proxy,
            Some(CarrierUpstreamProxy::Socks5 {
                host: "127.0.0.1".to_string(),
                port: 1080,
                auth: None,
            })
        );
    }

    #[test]
    fn parses_authenticated_directsock_upstream_proxy() {
        let endpoint = TrojanEndpoint::parse(
            "trojan://secret@example.com:443?security=tls&type=ws&upstream=http%3A%2F%2Fpacket%3Abridgepass%4010.0.0.2%3A18080",
        )
        .unwrap();

        assert_eq!(
            endpoint.upstream_proxy,
            Some(CarrierUpstreamProxy::Http {
                host: "10.0.0.2".to_string(),
                port: 18080,
                auth: Some(CarrierUpstreamAuth {
                    username: "packet".to_string(),
                    password: "bridgepass".to_string(),
                }),
            })
        );
    }

    #[test]
    fn hashes_trojan_password_as_sha224_hex() {
        assert_eq!(trojan_password_hash("humanity").len(), 56);
    }

    #[test]
    fn encodes_domain_destination() {
        let encoded = encode_trojan_address("example.com:443").unwrap();
        assert_eq!(encoded[0], 0x03);
        assert_eq!(encoded[1], "example.com".len() as u8);
        assert_eq!(&encoded[2..13], b"example.com");
        assert_eq!(&encoded[13..], &443u16.to_be_bytes());
    }

    #[tokio::test]
    async fn parses_socks5_domain_connect_request() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut version = [0u8; 1];
            stream.read_exact(&mut version).await.unwrap();
            assert_eq!(version[0], 0x05);
            read_socks5_connect_request(&mut stream).await.unwrap()
        });

        let mut client = TcpStream::connect(addr).await.unwrap();
        client.write_all(&[0x05, 0x01, 0x00]).await.unwrap();
        let mut method = [0u8; 2];
        client.read_exact(&mut method).await.unwrap();
        assert_eq!(method, [0x05, 0x00]);

        let domain = b"example.com";
        let mut request = vec![0x05, 0x01, 0x00, 0x03, domain.len() as u8];
        request.extend_from_slice(domain);
        request.extend_from_slice(&443u16.to_be_bytes());
        client.write_all(&request).await.unwrap();

        assert_eq!(server.await.unwrap(), "example.com:443");
    }
}
