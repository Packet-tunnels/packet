// obfs.rs — OSSH-style "looks like uniform random bytes" stream obfuscation
//
// WHY THIS EXISTS
// ───────────────
// Measured behaviour of Iran's filter (2026-05-18, residential LTE):
//   * Most foreign IPs are blackholed at TCP (no SYN/ACK).
//   * For IPs that DO reach TCP, ANY TLS ClientHello to a foreign endpoint
//     is RST-injected, regardless of SNI — it triggers on the recognisable
//     "TLS handshake leaving the country" pattern, not on an SNI blocklist.
//   * A connection that carries no ClientHello and no protocol structure has
//     nothing for that classifier to fire on.
//
// So the escape primitive is: from the very first byte, make the wire look
// like uniform random noise. No TLS record headers, no SNI, no HTTP verbs,
// no fixed-length handshake. This is the well-known obfuscated-SSH ("OSSH")
// technique, reimplemented here from its public description so it stays
// license-clean and dependency-light.
//
// This lives in phantom-proto so the client transport and the server
// listener share one implementation.
//
// SECURITY MODEL
// ──────────────
// This layer is an *obfuscator*, not the security boundary. Its only job is
// to defeat passive/active pattern classifiers. Confidentiality + integrity
// are still provided by phantom's existing inner frame crypto (XChaCha20-
// Poly1305 via `encrypt`/`decrypt`), which runs *inside* this stream. The
// pre-shared `obfs_key` is a low-entropy DPI-evasion secret (a "knock"),
// not a session key.
//
// WIRE FORMAT
// ───────────
//   client → server, once, at stream start:
//     [ seed: 16 random bytes, sent in clear ]      (random ⇒ no tell)
//     [ obfuscated: u16 pad_len | u16 magic | pad_len random bytes ]
//   thereafter, both directions: every byte XOR'd with a per-direction
//   SHA-256 counter keystream:
//     K_dir = SHA256( obfs_key || seed || dir_tag || counter_be_u64 )
//   concatenated over counter = 0,1,2,...  dir_tag = b"C2S" / b"S2C".
// `magic` lets the server reject non-obfs / wrong-key connections fast.
//
// Phantom's framed protocol runs on top via `write_msg`/`read_msg`, which
// add a u32-LE length prefix so messages survive a boundary-less TCP stream
// (WebSocket gave us message boundaries for free; raw TCP does not).

use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use rand::rngs::SmallRng;
use rand::{Rng, SeedableRng};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};

const SEED_LEN: usize = 16;
const MAGIC: u16 = 0x5048; // "PH"
const MAX_PAD: u16 = 700; // handshake size jitter; well under one MTU burst
/// Hard cap on a single framed message (16 MiB) so a corrupt/hostile length
/// prefix can't make us allocate unbounded memory.
const MAX_MSG_LEN: u32 = 16 * 1024 * 1024;

/// A SHA-256 counter-mode keystream: a uniform pseudo-random byte source
/// keyed by (obfs_key, seed, direction). Not an AEAD — purely for making
/// the wire look random.
struct KeyStream {
    key: Vec<u8>,
    seed: [u8; SEED_LEN],
    dir_tag: &'static [u8],
    counter: u64,
    block: Vec<u8>,
    block_pos: usize,
}

impl KeyStream {
    fn new(key: &[u8], seed: &[u8; SEED_LEN], dir_tag: &'static [u8]) -> Self {
        Self {
            key: key.to_vec(),
            seed: *seed,
            dir_tag,
            counter: 0,
            block: Vec::new(),
            block_pos: 0,
        }
    }

    fn refill(&mut self) {
        let mut h = Sha256::new();
        h.update(&self.key);
        h.update(self.seed);
        h.update(self.dir_tag);
        h.update(self.counter.to_be_bytes());
        self.block = h.finalize().to_vec();
        self.block_pos = 0;
        self.counter = self.counter.wrapping_add(1);
    }

    /// XOR `buf` in place with the next keystream bytes.
    fn apply(&mut self, buf: &mut [u8]) {
        for b in buf.iter_mut() {
            if self.block_pos >= self.block.len() {
                self.refill();
            }
            *b ^= self.block[self.block_pos];
            self.block_pos += 1;
        }
    }

    /// Move the keystream position back by `n` bytes (recover from a partial
    /// inner write). Safe because the keystream is deterministic.
    fn rewind(&mut self, mut n: usize) {
        while n > 0 {
            if self.block_pos == 0 {
                self.counter = self.counter.wrapping_sub(2);
                self.refill();
                self.block_pos = self.block.len();
            }
            let step = n.min(self.block_pos);
            self.block_pos -= step;
            n -= step;
        }
    }
}

/// A stream wrapped so every byte on the wire is XOR'd with a per-direction
/// keystream. The obfuscation handshake is completed by `connect_client` /
/// `accept_server` before this is returned, so the duplex path is a pure
/// transparent transform.
pub struct ObfsStream<S> {
    inner: S,
    tx: KeyStream,
    rx: KeyStream,
}

impl<S> ObfsStream<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    /// Client side: random seed in clear, then obfuscated header + random
    /// padding.
    pub async fn connect_client(mut inner: S, obfs_key: &[u8]) -> io::Result<Self> {
        let mut rng = SmallRng::from_entropy();
        let mut seed = [0u8; SEED_LEN];
        rng.fill(&mut seed);

        let mut tx = KeyStream::new(obfs_key, &seed, b"C2S");
        let rx = KeyStream::new(obfs_key, &seed, b"S2C");

        let pad_len: u16 = rng.gen_range(0..=MAX_PAD);
        let mut header = Vec::with_capacity(4 + pad_len as usize);
        header.extend_from_slice(&pad_len.to_be_bytes());
        header.extend_from_slice(&MAGIC.to_be_bytes());
        let mut pad = vec![0u8; pad_len as usize];
        rng.fill(pad.as_mut_slice());
        header.extend_from_slice(&pad);
        tx.apply(&mut header);

        inner.write_all(&seed).await?;
        inner.write_all(&header).await?;
        inner.flush().await?;

        Ok(Self { inner, tx, rx })
    }

    /// Server side: read seed, verify magic (so a non-obfs/wrong-key
    /// connection fails fast), consume padding.
    pub async fn accept_server(mut inner: S, obfs_key: &[u8]) -> io::Result<Self> {
        let mut seed = [0u8; SEED_LEN];
        inner.read_exact(&mut seed).await?;

        let mut rx = KeyStream::new(obfs_key, &seed, b"C2S");
        let tx = KeyStream::new(obfs_key, &seed, b"S2C");

        let mut head = [0u8; 4];
        inner.read_exact(&mut head).await?;
        rx.apply(&mut head);
        let pad_len = u16::from_be_bytes([head[0], head[1]]);
        let magic = u16::from_be_bytes([head[2], head[3]]);
        if magic != MAGIC {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "obfs: magic mismatch (not an obfs connection / wrong key)",
            ));
        }
        if pad_len > MAX_PAD {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "obfs: pad length out of range",
            ));
        }
        if pad_len > 0 {
            let mut pad = vec![0u8; pad_len as usize];
            inner.read_exact(&mut pad).await?;
            rx.apply(&mut pad);
        }

        Ok(Self { inner, tx, rx })
    }

}

/// Write one length-delimited message: `[u32 LE len][payload]`. Free
/// function so it works both on a whole `ObfsStream` (during the pre-split
/// auth exchange) and on a `tokio::io::split` write half (during the
/// concurrent bidirectional relay). Carries phantom's auth JSON and
/// encrypted frame batches over the raw, boundary-less obfuscated stream.
pub async fn write_msg<W: AsyncWrite + Unpin>(w: &mut W, payload: &[u8]) -> io::Result<()> {
    if payload.len() as u64 > MAX_MSG_LEN as u64 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "obfs: message exceeds MAX_MSG_LEN",
        ));
    }
    let len = (payload.len() as u32).to_le_bytes();
    w.write_all(&len).await?;
    w.write_all(payload).await?;
    w.flush().await?;
    Ok(())
}

/// Read one length-delimited message written by [`write_msg`].
pub async fn read_msg<R: AsyncRead + Unpin>(r: &mut R) -> io::Result<Vec<u8>> {
    let mut len_buf = [0u8; 4];
    r.read_exact(&mut len_buf).await?;
    let len = u32::from_le_bytes(len_buf);
    if len > MAX_MSG_LEN {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "obfs: incoming message length exceeds MAX_MSG_LEN",
        ));
    }
    let mut buf = vec![0u8; len as usize];
    r.read_exact(&mut buf).await?;
    Ok(buf)
}

impl<S: AsyncRead + Unpin> AsyncRead for ObfsStream<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let before = buf.filled().len();
        match Pin::new(&mut this.inner).poll_read(cx, buf) {
            Poll::Ready(Ok(())) => {
                let after = buf.filled().len();
                if after > before {
                    let region = &mut buf.filled_mut()[before..after];
                    this.rx.apply(region);
                }
                Poll::Ready(Ok(()))
            }
            other => other,
        }
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for ObfsStream<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        let mut scratch = buf.to_vec();
        this.tx.apply(&mut scratch);
        match Pin::new(&mut this.inner).poll_write(cx, &scratch) {
            Poll::Ready(Ok(n)) => {
                if n < scratch.len() {
                    this.tx.rewind(scratch.len() - n);
                }
                Poll::Ready(Ok(n))
            }
            other => other,
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn obfs_round_trip_and_framed_msgs() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let key = b"low-entropy-knock";

        let server = tokio::spawn(async move {
            let (sock, _) = listener.accept().await.unwrap();
            let mut s = ObfsStream::accept_server(sock, key).await.unwrap();
            let m = read_msg(&mut s).await.unwrap();
            assert_eq!(&m, b"hello world");
            write_msg(&mut s, b"pong-12345").await.unwrap();
        });

        let sock = tokio::net::TcpStream::connect(addr).await.unwrap();
        let mut c = ObfsStream::connect_client(sock, key).await.unwrap();
        write_msg(&mut c, b"hello world").await.unwrap();
        let back = read_msg(&mut c).await.unwrap();
        assert_eq!(&back, b"pong-12345");
        server.await.unwrap();
    }

    #[tokio::test]
    async fn wrong_key_is_rejected() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let server = tokio::spawn(async move {
            let (sock, _) = listener.accept().await.unwrap();
            ObfsStream::accept_server(sock, b"server-key").await.map(|_| ())
        });

        let sock = tokio::net::TcpStream::connect(addr).await.unwrap();
        let mut c = ObfsStream::connect_client(sock, b"client-key")
            .await
            .unwrap();
        let _ = write_msg(&mut c, b"x").await;

        let res = server.await.unwrap();
        assert!(res.is_err(), "server must reject a wrong-key handshake");
    }
}
