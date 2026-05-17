// phantom-proto: Shared protocol library for Phantom Tunnel
//
// This crate implements:
// - XChaCha20-Poly1305 encryption for tunnel data
// - HMAC-SHA256 authentication for session establishment
// - Binary frame encoding/decoding for multiplexed streams
// - HTTP request/response types for the covert channel

use chacha20poly1305::{
    aead::{Aead, KeyInit, OsRng},
    XChaCha20Poly1305, XNonce,
};
use hmac::{Hmac, Mac};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::time::{SystemTime, UNIX_EPOCH};

pub mod fragment;
pub mod onion;

type HmacSha256 = Hmac<Sha256>;

// ─── Stream Commands ───────────────────────────────────────────
/// Commands that flow between client and server inside the tunnel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Cmd {
    /// Client→Server: open a TCP connection to addr:port
    Connect = 1,
    /// Server→Client: connection established successfully
    ConnectOk = 2,
    /// Server→Client: connection failed
    ConnectErr = 3,
    /// Bidirectional: stream payload data
    Data = 4,
    /// Bidirectional: close a stream
    Close = 5,
    /// Bidirectional: relay-control metadata
    Relay = 6,
    /// Bidirectional: protocol-level fragment payload
    Fragment = 7,
    /// Bidirectional: synthetic cover traffic
    CoverTraffic = 8,
}

impl Cmd {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            1 => Some(Self::Connect),
            2 => Some(Self::ConnectOk),
            3 => Some(Self::ConnectErr),
            4 => Some(Self::Data),
            5 => Some(Self::Close),
            6 => Some(Self::Relay),
            7 => Some(Self::Fragment),
            8 => Some(Self::CoverTraffic),
            _ => None,
        }
    }
}

// ─── Frame ─────────────────────────────────────────────────────
/// A single tunneled frame carrying data for one stream.
#[derive(Debug, Clone)]
pub struct Frame {
    pub stream_id: u32,
    pub cmd: Cmd,
    pub data: Vec<u8>,
}

impl Frame {
    pub fn connect(stream_id: u32, addr: &str) -> Self {
        Self {
            stream_id,
            cmd: Cmd::Connect,
            data: addr.as_bytes().to_vec(),
        }
    }

    pub fn connect_ok(stream_id: u32) -> Self {
        Self {
            stream_id,
            cmd: Cmd::ConnectOk,
            data: vec![],
        }
    }

    pub fn connect_err(stream_id: u32, reason: &str) -> Self {
        Self {
            stream_id,
            cmd: Cmd::ConnectErr,
            data: reason.as_bytes().to_vec(),
        }
    }

    pub fn data(stream_id: u32, payload: Vec<u8>) -> Self {
        Self {
            stream_id,
            cmd: Cmd::Data,
            data: payload,
        }
    }

    pub fn close(stream_id: u32) -> Self {
        Self {
            stream_id,
            cmd: Cmd::Close,
            data: vec![],
        }
    }

    pub fn relay(stream_id: u32, payload: Vec<u8>) -> Self {
        Self {
            stream_id,
            cmd: Cmd::Relay,
            data: payload,
        }
    }

    pub fn fragment(stream_id: u32, payload: Vec<u8>) -> Self {
        Self {
            stream_id,
            cmd: Cmd::Fragment,
            data: payload,
        }
    }

    pub fn cover(stream_id: u32, payload: Vec<u8>) -> Self {
        Self {
            stream_id,
            cmd: Cmd::CoverTraffic,
            data: payload,
        }
    }
}

// ─── Binary Wire Format ────────────────────────────────────────
// TunnelMessage = [frame_count: u16 LE] [Frame]*
// Frame = [stream_id: u32 LE] [cmd: u8] [data_len: u16 LE] [data: bytes]

/// Encode multiple frames into a binary tunnel message.
pub fn encode_frames(frames: &[Frame]) -> Vec<u8> {
    let mut buf = Vec::new();
    let count = frames.len().min(u16::MAX as usize) as u16;
    buf.extend_from_slice(&count.to_le_bytes());

    for frame in frames.iter().take(count as usize) {
        buf.extend_from_slice(&frame.stream_id.to_le_bytes());
        buf.push(frame.cmd as u8);
        let data_len = frame.data.len().min(u16::MAX as usize) as u16;
        buf.extend_from_slice(&data_len.to_le_bytes());
        buf.extend_from_slice(&frame.data[..data_len as usize]);
    }
    buf
}

/// Decode a binary tunnel message into frames.
pub fn decode_frames(data: &[u8]) -> Result<Vec<Frame>, &'static str> {
    if data.len() < 2 {
        return Ok(vec![]);
    }
    let count = u16::from_le_bytes([data[0], data[1]]) as usize;
    let mut frames = Vec::with_capacity(count);
    let mut pos = 2;

    for _ in 0..count {
        if pos + 7 > data.len() {
            return Err("truncated frame header");
        }
        let stream_id =
            u32::from_le_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]);
        let cmd = Cmd::from_u8(data[pos + 4]).ok_or("invalid command")?;
        let data_len = u16::from_le_bytes([data[pos + 5], data[pos + 6]]) as usize;
        pos += 7;

        if pos + data_len > data.len() {
            return Err("truncated frame data");
        }
        let payload = data[pos..pos + data_len].to_vec();
        pos += data_len;

        frames.push(Frame {
            stream_id,
            cmd,
            data: payload,
        });
    }
    Ok(frames)
}

// ─── Crypto ────────────────────────────────────────────────────

/// Derive a 32-byte encryption key from the shared secret using HMAC.
pub fn derive_key(secret: &str) -> [u8; 32] {
    let mut mac = <HmacSha256 as Mac>::new_from_slice(b"phantom-tunnel-key-derivation").unwrap();
    mac.update(secret.as_bytes());
    let result = mac.finalize().into_bytes();
    let mut key = [0u8; 32];
    key.copy_from_slice(&result);
    key
}

/// Encrypt a plaintext payload.
/// Returns: [24-byte nonce] [ciphertext + 16-byte tag]
pub fn encrypt(key: &[u8; 32], plaintext: &[u8]) -> Vec<u8> {
    let cipher = XChaCha20Poly1305::new(key.into());
    let mut nonce_bytes = [0u8; 24];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = XNonce::from_slice(&nonce_bytes);
    let ciphertext = cipher.encrypt(nonce, plaintext).expect("encryption failed");
    let mut out = Vec::with_capacity(24 + ciphertext.len());
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ciphertext);
    out
}

/// Decrypt a payload produced by `encrypt`.
pub fn decrypt(key: &[u8; 32], data: &[u8]) -> Result<Vec<u8>, &'static str> {
    if data.len() < 24 {
        return Err("ciphertext too short");
    }
    let cipher = XChaCha20Poly1305::new(key.into());
    let nonce = XNonce::from_slice(&data[..24]);
    cipher
        .decrypt(nonce, &data[24..])
        .map_err(|_| "decryption failed")
}

// ─── Authentication ────────────────────────────────────────────

fn auth_mac(secret: &str, timestamp: u64, nonce: &str) -> HmacSha256 {
    let mut mac = <HmacSha256 as Mac>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(timestamp.to_string().as_bytes());
    mac.update(b":");
    mac.update(nonce.as_bytes());
    mac
}

/// Generate an authentication signature for the given timestamp + nonce pair.
pub fn sign_auth(secret: &str, timestamp: u64, nonce: &str) -> String {
    hex::encode(&auth_mac(secret, timestamp, nonce).finalize().into_bytes())
}

/// Verify an authentication signature.
pub fn verify_auth(secret: &str, timestamp: u64, nonce: &str, signature: &str) -> bool {
    if nonce.is_empty() {
        return false;
    }

    let signature_bytes = match hex::decode(signature) {
        Ok(bytes) => bytes,
        Err(_) => return false,
    };

    auth_mac(secret, timestamp, nonce)
        .verify_slice(&signature_bytes)
        .is_ok()
}

/// Generate a random authentication nonce to prevent replay.
pub fn generate_auth_nonce() -> String {
    let mut bytes = [0u8; 16];
    OsRng.fill_bytes(&mut bytes);
    hex::encode(&bytes)
}

/// Build an auth request with a fresh timestamp + nonce.
pub fn build_auth_request(secret: &str) -> AuthRequest {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let n = generate_auth_nonce();
    let sig = sign_auth(secret, ts, &n);
    AuthRequest {
        ts,
        n,
        sig,
        ticket: None,
        mode: None,
        label: None,
    }
}

pub fn build_ticket_auth_request(
    ticket: impl Into<String>,
    mode: Option<String>,
    label: Option<String>,
) -> AuthRequest {
    AuthRequest {
        ts: unix_now_secs(),
        n: generate_auth_nonce(),
        sig: String::new(),
        ticket: Some(ticket.into()),
        mode,
        label,
    }
}

/// Generate a random session token.
pub fn generate_session_token() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    hex::encode(&bytes)
}

// ─── HTTP Types ────────────────────────────────────────────────
// These are the JSON bodies exchanged over HTTP, designed to look
// like a normal web application API.

#[derive(Serialize, Deserialize)]
pub struct AuthRequest {
    /// Unix timestamp
    pub ts: u64,
    /// Per-request nonce to prevent auth replay
    #[serde(default)]
    pub n: String,
    /// HMAC-SHA256 signature
    #[serde(default)]
    pub sig: String,
    /// Short-lived signed transport ticket
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ticket: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub struct AuthResponse {
    pub token: String,
}

#[derive(Serialize, Deserialize)]
pub struct SyncRequest {
    /// Session token
    pub t: String,
    /// Base64-encoded encrypted tunnel message
    pub d: String,
}

#[derive(Serialize, Deserialize)]
pub struct SyncResponse {
    /// Base64-encoded encrypted tunnel message
    pub d: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TransportTicketClaims {
    pub sub: String,
    pub iat: u64,
    pub exp: u64,
    pub jti: String,
    pub session_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bridge_id: Option<String>,
    #[serde(default)]
    pub capabilities: Vec<String>,
}

impl TransportTicketClaims {
    pub fn session_key_bytes(&self) -> Result<[u8; 32], &'static str> {
        let bytes = b64_decode(&self.session_key)?;
        if bytes.len() != 32 {
            return Err("invalid ticket session key length");
        }
        let mut key = [0u8; 32];
        key.copy_from_slice(&bytes);
        Ok(key)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct PacketBridgeDescriptor {
    pub id: String,
    pub base_url: String,
    #[serde(default)]
    pub spki_pins: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<u64>,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct PacketPeerDescriptor {
    pub peer_id: String,
    #[serde(default)]
    pub relay_urls: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trust_score: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_seen_at: Option<u64>,
    #[serde(default)]
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct MeshBootstrapConfig {
    pub ticket: String,
    #[serde(default)]
    pub bridges: Vec<PacketBridgeDescriptor>,
    #[serde(default)]
    pub peers: Vec<PacketPeerDescriptor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preferred_bridge_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct MeshStatsSnapshot {
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_bridge_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_port: Option<u16>,
    #[serde(default)]
    pub known_peers: usize,
    #[serde(default)]
    pub trusted_peers: usize,
    #[serde(default)]
    pub imported_bridges: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

pub fn issue_transport_ticket(
    secret: &str,
    subject: &str,
    ttl_secs: u64,
    bridge_id: Option<String>,
    capabilities: Vec<String>,
) -> String {
    let now = unix_now_secs();
    let mut session_key = [0u8; 32];
    OsRng.fill_bytes(&mut session_key);
    let claims = TransportTicketClaims {
        sub: subject.to_string(),
        iat: now,
        exp: now.saturating_add(ttl_secs.max(1)),
        jti: generate_auth_nonce(),
        session_key: b64_encode(&session_key),
        bridge_id,
        capabilities,
    };
    sign_transport_ticket(secret, &claims)
}

pub fn sign_transport_ticket(secret: &str, claims: &TransportTicketClaims) -> String {
    let payload = serde_json::to_vec(claims).expect("ticket claims serialization failed");
    let encoded_payload = b64_encode(&payload);
    let mut mac = <HmacSha256 as Mac>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(encoded_payload.as_bytes());
    let signature = hex::encode(&mac.finalize().into_bytes());
    format!("{}.{}", encoded_payload, signature)
}

pub fn decode_transport_ticket(token: &str) -> Result<TransportTicketClaims, &'static str> {
    let (payload, _sig) = split_transport_ticket(token)?;
    let decoded_payload = b64_decode(payload)?;
    serde_json::from_slice(&decoded_payload).map_err(|_| "invalid transport ticket payload")
}

pub fn verify_transport_ticket(
    secret: &str,
    token: &str,
    now: u64,
) -> Result<TransportTicketClaims, &'static str> {
    let (payload, signature) = split_transport_ticket(token)?;
    let mut mac = <HmacSha256 as Mac>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(payload.as_bytes());
    let signature_bytes = hex::decode(signature)?;
    mac.verify_slice(&signature_bytes)
        .map_err(|_| "invalid transport ticket signature")?;

    let claims = decode_transport_ticket(token)?;
    if claims.exp <= now {
        return Err("transport ticket expired");
    }
    if claims.iat > now.saturating_add(30) {
        return Err("transport ticket issued in the future");
    }
    let _ = claims.session_key_bytes()?;
    Ok(claims)
}

fn split_transport_ticket(token: &str) -> Result<(&str, &str), &'static str> {
    token
        .split_once('.')
        .ok_or("invalid transport ticket format")
}

// ─── Hex helpers (minimal, to avoid another dep) ───────────────
mod hex {
    pub fn encode(data: &[u8]) -> String {
        data.iter().map(|b| format!("{:02x}", b)).collect()
    }

    pub fn decode(value: &str) -> Result<Vec<u8>, &'static str> {
        if value.len() % 2 != 0 {
            return Err("invalid hex");
        }

        let mut decoded = Vec::with_capacity(value.len() / 2);
        let bytes = value.as_bytes();
        let mut index = 0usize;

        while index < bytes.len() {
            let high = decode_nibble(bytes[index])?;
            let low = decode_nibble(bytes[index + 1])?;
            decoded.push((high << 4) | low);
            index += 2;
        }

        Ok(decoded)
    }

    fn decode_nibble(byte: u8) -> Result<u8, &'static str> {
        match byte {
            b'0'..=b'9' => Ok(byte - b'0'),
            b'a'..=b'f' => Ok(byte - b'a' + 10),
            b'A'..=b'F' => Ok(byte - b'A' + 10),
            _ => Err("invalid hex"),
        }
    }
}

pub fn sha256(data: &[u8]) -> [u8; 32] {
    use sha2::Digest;

    let digest = Sha256::digest(data);
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    out
}

// ─── Base64 helpers ────────────────────────────────────────────
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;

pub fn b64_encode(data: &[u8]) -> String {
    B64.encode(data)
}

pub fn b64_decode(s: &str) -> Result<Vec<u8>, &'static str> {
    B64.decode(s).map_err(|_| "invalid base64")
}

pub fn unix_now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

// ─── Traffic Padding ───────────────────────────────────────────
// Pad payloads to fixed block sizes to prevent traffic analysis.
// DPI can fingerprint tunnel protocols by message size distribution.
// Padding normalizes sizes so traffic looks like a standard web app.

/// Pad a payload to a multiple of `block_size` with random bytes.
/// Format: [original_len: u32 LE] [payload] [random padding to block boundary]
pub fn pad_payload(data: &[u8], block_size: usize) -> Vec<u8> {
    let block_size = block_size.max(64); // minimum 64 byte blocks
    let total_needed = 4 + data.len(); // 4 bytes for length prefix
    let padded_len = ((total_needed + block_size - 1) / block_size) * block_size;

    let mut out = Vec::with_capacity(padded_len);
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(data);

    // Fill remaining with random bytes
    let pad_len = padded_len - total_needed;
    if pad_len > 0 {
        let mut padding = vec![0u8; pad_len];
        OsRng.fill_bytes(&mut padding);
        out.extend_from_slice(&padding);
    }
    out
}

/// Remove padding from a padded payload.
pub fn unpad_payload(data: &[u8]) -> Result<Vec<u8>, &'static str> {
    if data.len() < 4 {
        return Err("padded payload too short");
    }
    let len = u32::from_le_bytes([data[0], data[1], data[2], data[3]]) as usize;
    if 4 + len > data.len() {
        return Err("invalid padding length");
    }
    Ok(data[4..4 + len].to_vec())
}

// ─── Tests ─────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encrypt_decrypt() {
        let key = derive_key("test-secret");
        let plaintext = b"hello world";
        let encrypted = encrypt(&key, plaintext);
        let decrypted = decrypt(&key, &encrypted).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_frame_roundtrip() {
        let frames = vec![
            Frame::connect(1, "google.com:443"),
            Frame::data(2, b"hello".to_vec()),
            Frame::close(3),
        ];
        let encoded = encode_frames(&frames);
        let decoded = decode_frames(&encoded).unwrap();
        assert_eq!(decoded.len(), 3);
        assert_eq!(decoded[0].stream_id, 1);
        assert_eq!(decoded[0].cmd, Cmd::Connect);
        assert_eq!(decoded[1].cmd, Cmd::Data);
        assert_eq!(decoded[2].cmd, Cmd::Close);
    }

    #[test]
    fn test_auth() {
        let secret = "my-secret";
        let ts = 1234567890u64;
        let nonce = "6d5cc6b7cbbd4b4ab7a6fc907f9455c1";
        let sig = sign_auth(secret, ts, nonce);
        assert!(verify_auth(secret, ts, nonce, &sig));
        assert!(!verify_auth(secret, ts + 1, nonce, &sig));
        assert!(!verify_auth(secret, ts, "different", &sig));
        assert!(!verify_auth(secret, ts, "", &sig));
    }

    #[test]
    fn test_ticket_roundtrip_and_expiry() {
        let secret = "mesh-secret";
        let ticket = issue_transport_ticket(
            secret,
            "user-123",
            60,
            Some("bridge-1".to_string()),
            vec!["mesh".to_string()],
        );
        let claims = verify_transport_ticket(secret, &ticket, unix_now_secs()).unwrap();
        assert_eq!(claims.sub, "user-123");
        assert_eq!(claims.bridge_id.as_deref(), Some("bridge-1"));
        assert_eq!(claims.capabilities, vec!["mesh".to_string()]);
        assert_eq!(claims.session_key_bytes().unwrap().len(), 32);

        let expired = TransportTicketClaims {
            sub: "user-123".to_string(),
            iat: 10,
            exp: 20,
            jti: "expired".to_string(),
            session_key: b64_encode(&[7u8; 32]),
            bridge_id: None,
            capabilities: vec![],
        };
        let expired_token = sign_transport_ticket(secret, &expired);
        assert_eq!(
            verify_transport_ticket(secret, &expired_token, 30).unwrap_err(),
            "transport ticket expired"
        );
    }

    #[test]
    fn test_onion_wrap_and_peel() {
        let payload = b"vibe-packet";
        let hop1 = derive_key("hop-1");
        let hop2 = derive_key("hop-2");
        let wrapped = onion::onion_wrap(payload, &[hop1, hop2]);
        let (remaining_1, first) = onion::onion_peel(&wrapped, &hop1).unwrap();
        let (remaining_2, second) = onion::onion_peel(&first, &hop2).unwrap();
        assert_eq!(remaining_1, 2);
        assert_eq!(remaining_2, 1);
        assert_eq!(second, payload);
    }

    #[test]
    fn test_k_of_n_fragment_reconstruction_and_corruption_handling() {
        let fragments = fragment::split_secret_shares(b"mesh-data", 3, 5).unwrap();
        let recovered = fragment::reconstruct_secret(&[
            fragments[0].clone(),
            fragments[2].clone(),
            fragments[4].clone(),
        ])
        .unwrap();
        assert_eq!(recovered, b"mesh-data");

        let mut corrupted = fragments[1].clone();
        corrupted.share_data[0] ^= 0x7f;
        let recovered_with_fallback = fragment::reconstruct_secret(&[
            fragments[0].clone(),
            corrupted,
            fragments[2].clone(),
            fragments[3].clone(),
        ])
        .unwrap();
        assert_eq!(recovered_with_fallback, b"mesh-data");
    }

    #[test]
    fn test_fragment_padding_roundtrips_to_standard_size() {
        let fragment = fragment::split_secret_shares(b"voice-note", 2, 3)
            .unwrap()
            .into_iter()
            .next()
            .unwrap();
        let padded = fragment.to_padded_bytes();
        assert!(fragment::STANDARD_SIZES.contains(&padded.len()));
        let decoded = fragment::KOfNFragment::from_padded_bytes(&padded).unwrap();
        assert_eq!(decoded, fragment);
    }

    #[test]
    fn test_fragment_rejects_duplicate_share_indices() {
        let fragments = fragment::split_secret_shares(b"small-image", 2, 3).unwrap();
        let error = fragment::reconstruct_secret(&[fragments[0].clone(), fragments[0].clone()])
            .unwrap_err();
        assert_eq!(error, "duplicate fragment share index");
    }
}
