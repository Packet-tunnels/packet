// tls_fragment.rs — Multi-segment randomized TLS ClientHello fragmentation
// with v2rayNG/Hiddify-style inter-segment delays.
//
// Iran's DPI extracts SNI from the TLS ClientHello to enforce per-domain
// blocklists. The current production filter (as observed on Iranian
// residential ISPs in 2026) does TCP reassembly across the first 4–8
// segments before parsing — so pure size-fragmentation alone (no delays)
// gets caught. The working v2rayNG/Hiddify "tlshello" preset that bypasses
// this filter today uses BOTH:
//
//   * randomized chunk sizes (100–150 bytes), and
//   * randomized inter-chunk delays (10–20 ms),
//
// because the random delays push later segments past the DPI's
// reassembly time window, while the random sizes prevent static-pattern
// fingerprinting of the fragmenter itself.
//
// Combined with TCP_NODELAY (so each write() = one TCP segment), this is
// the same exact technique any working Iranian VLESS/Trojan client uses
// today.

use rand::rngs::SmallRng;
use rand::{Rng, SeedableRng};
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
use tokio::time::Sleep;

/// Number of initial writes that get fragmented. The TLS ClientHello fits
/// in 1–2 logical writes from rustls; covering 5 absorbs whatever the
/// caller writes immediately afterwards (cipher change spec, finished).
const FORCED_FRAGMENT_WRITES: u8 = 5;

/// Chunk size range — matches v2rayNG `tlshello` preset.
const MIN_FRAGMENT_SIZE: usize = 100;
const MAX_FRAGMENT_SIZE: usize = 150;

/// Inter-segment delay range in milliseconds — matches v2rayNG preset.
const MIN_DELAY_MS: u64 = 10;
const MAX_DELAY_MS: u64 = 20;

pub struct FragmentStream {
    inner: TcpStream,
    /// Forced-fragment writes still to emit. 0 = passthrough.
    fragments_remaining: u8,
    rng: SmallRng,
    /// Configured base hint from the legacy CLI. When the caller passes
    /// a value inside the [MIN, MAX] window we honour it as the lower
    /// bound; otherwise we use [MIN, MAX] directly. 0 means passthrough.
    base_size: usize,
    /// Inter-segment delay that the previous write owes us before the
    /// next chunk goes out. Polled on the front of every poll_write so
    /// the random delay is enforced inside the async runtime.
    pending_sleep: Option<Pin<Box<Sleep>>>,
}

impl FragmentStream {
    pub fn new(stream: TcpStream, fragment_size: usize) -> Self {
        // Each individual write() must hit the wire as its own TCP segment.
        let _ = stream.set_nodelay(true);
        Self {
            inner: stream,
            fragments_remaining: FORCED_FRAGMENT_WRITES,
            rng: SmallRng::from_entropy(),
            base_size: fragment_size,
            pending_sleep: None,
        }
    }

    pub fn passthrough(stream: TcpStream) -> Self {
        Self {
            inner: stream,
            fragments_remaining: 0,
            rng: SmallRng::from_entropy(),
            base_size: 0,
            pending_sleep: None,
        }
    }

    /// Pick the next forced-fragment chunk size from the v2rayNG-style
    /// random range. `base_size` from the legacy CLI is used as the low
    /// edge when it falls inside the window; otherwise we just sample
    /// the default [100, 150].
    fn next_chunk_size(&mut self, buf_len: usize) -> usize {
        let lower = if self.base_size >= MIN_FRAGMENT_SIZE && self.base_size < MAX_FRAGMENT_SIZE {
            self.base_size
        } else {
            MIN_FRAGMENT_SIZE
        };
        let upper = MAX_FRAGMENT_SIZE.max(lower + 1);
        let pick = self.rng.gen_range(lower..=upper);
        pick.min(buf_len).max(1)
    }

    fn next_delay_ms(&mut self) -> u64 {
        self.rng.gen_range(MIN_DELAY_MS..=MAX_DELAY_MS)
    }
}

impl AsyncWrite for FragmentStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let this = self.get_mut();

        // 1) Finish any pending inter-segment delay before sending the
        //    next chunk. This is what defeats stateful reassemblers.
        if let Some(sleep) = this.pending_sleep.as_mut() {
            match sleep.as_mut().poll(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(()) => {
                    this.pending_sleep = None;
                }
            }
        }

        // 2) Once fragmentation is done, pass through directly.
        if this.fragments_remaining == 0 || buf.is_empty() {
            return Pin::new(&mut this.inner).poll_write(cx, buf);
        }

        // 3) Emit one randomly-sized chunk.
        let chunk = this.next_chunk_size(buf.len());
        let slice = &buf[..chunk];
        let result = Pin::new(&mut this.inner).poll_write(cx, slice);

        if let Poll::Ready(Ok(n)) = &result {
            if *n > 0 {
                this.fragments_remaining = this.fragments_remaining.saturating_sub(1);

                // 4) Arm a randomized delay before the next write, but
                //    only if more fragmentation is still scheduled.
                if this.fragments_remaining > 0 {
                    let ms = this.next_delay_ms();
                    this.pending_sleep =
                        Some(Box::pin(tokio::time::sleep(Duration::from_millis(ms))));
                }
            }
        }
        result
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
