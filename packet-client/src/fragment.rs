// fragment.rs — TLS ClientHello TCP fragmentation
//
// Iran's DPI inspects the first TCP segment of TLS connections
// to extract the SNI (Server Name Indication) field from the
// TLS ClientHello message. If the SNI matches a blocked domain,
// the connection is terminated via TCP RST injection.
//
// This module implements TCP-level fragmentation that splits
// the first write (ClientHello) across multiple TCP segments,
// preventing DPI from extracting the SNI in a single pass.
//
// Technique (same as GoodbyeDPI):
// 1. Disable Nagle's algorithm (TCP_NODELAY) for precise segment control
// 2. On the first write (ClientHello), only send `fragment_size` bytes
// 3. The TLS library sees a partial write and retries with remaining data
// 4. Each partial write becomes a separate TCP segment
// 5. DPI only inspects the first segment — SNI is in a later segment
//
// Typical fragment_size: 40 bytes (SNI field starts around byte 40-60)

use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;

/// Fragment state machine for the first two writes.
enum FragState {
    /// First write: send only `fragment_size` bytes
    First,
    /// Second write: send another small chunk to further split
    Second,
    /// All subsequent writes: pass through directly
    Done,
}

/// A TCP stream wrapper that fragments the first write operations.
///
/// When the TLS library writes the ClientHello (typically 200-600 bytes),
/// this wrapper returns a "short write" of only `fragment_size` bytes.
/// The TLS library then retries with the remaining bytes, which go as
/// a separate TCP segment. This splits the SNI across packets.
///
/// For non-TLS (plain HTTP) connections, this also works to split
/// the HTTP request across segments, preventing Host header extraction.
pub struct FragmentStream {
    inner: TcpStream,
    state: FragState,
    fragment_size: usize,
}

impl FragmentStream {
    /// Create a new FragmentStream.
    ///
    /// `fragment_size`: bytes to send in the first TCP segment.
    /// Recommended: 40 for TLS (before SNI), 20 for HTTP (before Host).
    pub fn new(stream: TcpStream, fragment_size: usize) -> Self {
        // Disable Nagle's algorithm so each write() becomes a separate segment
        let _ = stream.set_nodelay(true);
        Self {
            inner: stream,
            state: FragState::First,
            fragment_size: fragment_size.max(10), // minimum 10 bytes
        }
    }

    /// Create a pass-through wrapper (no fragmentation).
    pub fn passthrough(stream: TcpStream) -> Self {
        Self {
            inner: stream,
            state: FragState::Done,
            fragment_size: 0,
        }
    }
}

impl AsyncWrite for FragmentStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let this = self.get_mut();

        match this.state {
            FragState::First => {
                if buf.len() > this.fragment_size {
                    // Only write fragment_size bytes — creates first TCP segment
                    let fragment = &buf[..this.fragment_size];
                    let result = Pin::new(&mut this.inner).poll_write(cx, fragment);
                    if let Poll::Ready(Ok(_)) = &result {
                        this.state = FragState::Second;
                    }
                    result
                } else {
                    // Buffer smaller than fragment — send as-is, move to next state
                    this.state = FragState::Second;
                    Pin::new(&mut this.inner).poll_write(cx, buf)
                }
            }
            FragState::Second => {
                // Second fragment: send another small chunk for extra splitting
                let chunk_size = this.fragment_size.min(buf.len());
                let result = Pin::new(&mut this.inner).poll_write(cx, &buf[..chunk_size]);
                if let Poll::Ready(Ok(_)) = &result {
                    this.state = FragState::Done;
                }
                result
            }
            FragState::Done => {
                // Normal pass-through for all subsequent writes
                Pin::new(&mut this.inner).poll_write(cx, buf)
            }
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}

impl AsyncRead for FragmentStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_read(cx, buf)
    }
}
