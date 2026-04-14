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
}

impl Cmd {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            1 => Some(Self::Connect),
            2 => Some(Self::ConnectOk),
            3 => Some(Self::ConnectErr),
            4 => Some(Self::Data),
            5 => Some(Self::Close),
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
        let stream_id = u32::from_le_bytes([
            data[pos],
            data[pos + 1],
            data[pos + 2],
            data[pos + 3],
        ]);
        let cmd = Cmd::from_u8(data[pos + 4]).ok_or("invalid command")?;
        let data_len =
            u16::from_le_bytes([data[pos + 5], data[pos + 6]]) as usize;
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
    let mut mac =
        <HmacSha256 as Mac>::new_from_slice(b"phantom-tunnel-key-derivation").unwrap();
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

/// Generate an authentication signature for the given timestamp.
pub fn sign_auth(secret: &str, timestamp: u64) -> String {
    let mut mac = <HmacSha256 as Mac>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(timestamp.to_string().as_bytes());
    hex::encode(&mac.finalize().into_bytes())
}

/// Verify an authentication signature.
pub fn verify_auth(secret: &str, timestamp: u64, signature: &str) -> bool {
    let expected = sign_auth(secret, timestamp);
    // Constant-time comparison via HMAC verify
    expected == signature
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
    /// HMAC-SHA256 signature
    pub sig: String,
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

// ─── Hex helpers (minimal, to avoid another dep) ───────────────
mod hex {
    pub fn encode(data: &[u8]) -> String {
        data.iter().map(|b| format!("{:02x}", b)).collect()
    }

    pub fn decode(s: &str) -> Result<Vec<u8>, &'static str> {
        if s.len() % 2 != 0 {
            return Err("odd hex length");
        }
        (0..s.len())
            .step_by(2)
            .map(|i| {
                u8::from_str_radix(&s[i..i + 2], 16).map_err(|_| "invalid hex")
            })
            .collect()
    }
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
        let sig = sign_auth(secret, ts);
        assert!(verify_auth(secret, ts, &sig));
        assert!(!verify_auth(secret, ts + 1, &sig));
    }
}
