// fragment.rs — TLS ClientHello TCP fragmentation
//
// Iran's DPI inspects the first TCP segment of TLS connections
// to extract the SNI (Server Name Indication) field from the
// TLS ClientHello message. If the SNI matches a blocked domain,
// the connection is terminated via TCP RST injection.
//
// This implementation matches the v2rayNG/sing-box "tlshello" fragment
// pattern proven to bypass Iran's DPI on residential connections:
//   - random chunk sizes in `len_range` (default 100-150 bytes)
//   - random inter-chunk delays in `delay_ms_range` (default 10-20 ms)
//   - fragments only the first `fragment_writes` writes (covers the
//     full TLS ClientHello), then passes everything through
//
// Fixed-size or low-count fragmentation has been fingerprinted by Iran's
// DPI; randomization and multi-segment splitting defeats the pattern
// matcher.

use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use rand::rngs::SmallRng;
use rand::{Rng, SeedableRng};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
use tokio::time::Sleep;

/// Tunables for the fragment stream. Defaults match the v2rayNG/Hiddify
/// preset that is currently working on Iranian residential ISPs.
#[derive(Clone, Debug)]
pub struct FragmentConfig {
    /// Inclusive range of bytes per fragmented write.
    pub len_range: (usize, usize),
    /// Inclusive range of milliseconds to sleep between fragments.
    pub delay_ms_range: (u64, u64),
    /// Number of initial writes to fragment. The TLS ClientHello fits
    /// in 1–2 writes from rustls; using 3–5 covers the case where the
    /// client sends additional handshake records (cipher change spec,
    /// finished) immediately after.
    pub fragment_writes: usize,
}

impl Default for FragmentConfig {
    fn default() -> Self {
        // v2rayNG defaults that are currently bypassing Iran DPI for
        // residential users with the "tlshello" preset.
        Self {
            len_range: (100, 150),
            delay_ms_range: (10, 20),
            fragment_writes: 5,
        }
    }
}

enum FragState {
    /// `writes_remaining` initial writes still get fragmented.
    Active { writes_remaining: usize },
    /// All subsequent writes pass through directly.
    Done,
}

pub struct FragmentStream {
    inner: TcpStream,
    state: FragState,
    cfg: FragmentConfig,
    rng: SmallRng,
    pending_sleep: Option<Pin<Box<Sleep>>>,
}

impl FragmentStream {
    /// New fragmenting wrapper using the supplied config.
    pub fn with_config(stream: TcpStream, cfg: FragmentConfig) -> Self {
        // Nagle off so each write produces a discrete TCP segment.
        let _ = stream.set_nodelay(true);
        let fragment_writes = cfg.fragment_writes.max(1);
        Self {
            inner: stream,
            state: FragState::Active {
                writes_remaining: fragment_writes,
            },
            cfg,
            rng: SmallRng::from_entropy(),
            pending_sleep: None,
        }
    }

    /// Backward-compatible constructor.
    ///
    /// Legacy callers passed a single fixed `fragment_size`. We map that
    /// to a tight range centred on that size so existing CLIs keep
    /// working, but the modern path is `with_config` + the default
    /// v2rayNG-style randomised settings.
    pub fn new(stream: TcpStream, fragment_size: usize) -> Self {
        let size = fragment_size.max(10);
        let cfg = FragmentConfig {
            len_range: (size, size),
            delay_ms_range: (0, 0),
            fragment_writes: 2,
        };
        Self::with_config(stream, cfg)
    }

    /// No-op wrapper (no fragmentation).
    pub fn passthrough(stream: TcpStream) -> Self {
        Self {
            inner: stream,
            state: FragState::Done,
            cfg: FragmentConfig::default(),
            rng: SmallRng::from_entropy(),
            pending_sleep: None,
        }
    }

    fn pick_len(&mut self, max: usize) -> usize {
        let (lo, hi) = self.cfg.len_range;
        let lo = lo.max(1);
        let hi = hi.max(lo);
        let pick = if lo == hi {
            lo
        } else {
            self.rng.gen_range(lo..=hi)
        };
        pick.min(max).max(1)
    }

    fn pick_delay_ms(&mut self) -> u64 {
        let (lo, hi) = self.cfg.delay_ms_range;
        if hi == 0 {
            return 0;
        }
        let hi = hi.max(lo);
        if lo == hi {
            lo
        } else {
            self.rng.gen_range(lo..=hi)
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

        // 1) If we owe a delay from the previous write, finish it first.
        if let Some(sleep) = this.pending_sleep.as_mut() {
            match sleep.as_mut().poll(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(()) => {
                    this.pending_sleep = None;
                }
            }
        }

        // 2) If fragmentation is done, just pass through.
        let writes_remaining = match this.state {
            FragState::Active { writes_remaining } => writes_remaining,
            FragState::Done => {
                return Pin::new(&mut this.inner).poll_write(cx, buf);
            }
        };

        // 3) Write a randomly-sized chunk of this buffer.
        let chunk_len = this.pick_len(buf.len());
        let result = Pin::new(&mut this.inner).poll_write(cx, &buf[..chunk_len]);

        if let Poll::Ready(Ok(n)) = &result {
            let still_to_fragment = writes_remaining.saturating_sub(1);
            this.state = if still_to_fragment == 0 {
                FragState::Done
            } else {
                FragState::Active {
                    writes_remaining: still_to_fragment,
                }
            };

            // Arm a delay before the next write if more fragmentation
            // is scheduled and we actually emitted bytes.
            if still_to_fragment > 0 && *n > 0 {
                let ms = this.pick_delay_ms();
                if ms > 0 {
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
