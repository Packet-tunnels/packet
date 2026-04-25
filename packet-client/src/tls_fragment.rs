use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;

enum FragState {
    First,
    Second,
    Done,
}

pub struct FragmentStream {
    inner: TcpStream,
    state: FragState,
    fragment_size: usize,
}

impl FragmentStream {
    pub fn new(stream: TcpStream, fragment_size: usize) -> Self {
        let _ = stream.set_nodelay(true);
        Self {
            inner: stream,
            state: FragState::First,
            fragment_size: fragment_size.max(10),
        }
    }

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
                    let fragment = &buf[..this.fragment_size];
                    let result = Pin::new(&mut this.inner).poll_write(cx, fragment);
                    if let Poll::Ready(Ok(_)) = &result {
                        this.state = FragState::Second;
                    }
                    result
                } else {
                    this.state = FragState::Second;
                    Pin::new(&mut this.inner).poll_write(cx, buf)
                }
            }
            FragState::Second => {
                let chunk_size = this.fragment_size.min(buf.len());
                let result = Pin::new(&mut this.inner).poll_write(cx, &buf[..chunk_size]);
                if let Poll::Ready(Ok(_)) = &result {
                    this.state = FragState::Done;
                }
                result
            }
            FragState::Done => Pin::new(&mut this.inner).poll_write(cx, buf),
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
