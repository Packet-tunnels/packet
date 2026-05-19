// chrome_tls.rs — Chrome-identical TLS via BoringSSL
//
// rustls emits a fixed, well-known ClientHello. Iran's 2026 DPI RSTs that
// JA3 (our Iran diagnostic: every rustls handshake = RST-AFTER-CLIENTHELLO),
// while the *same* trojan config works in v2ray because v2ray uses uTLS to
// send a byte-identical Chrome ClientHello.
//
// BoringSSL is the TLS library Chrome itself ships. With GREASE enabled and
// Chrome's cipher / curve / signature-algorithm ordering, the ClientHello it
// produces is JA3-indistinguishable from real Chrome — exactly what v2ray's
// `fp=chrome` does. We splice this in front of the WebSocket carrier so the
// handshake Iran sees is "a browser opening an HTTPS site", not "a proxy".
//
// TLS here is pure DPI camouflage. The phantom protocol inside the tunnel has
// its own authentication + encryption (shared secret), so we deliberately do
// NOT verify the server certificate: it avoids a mobile root-store
// cross-compile dependency and a cert mismatch when fronting through a CDN to
// our own origin. Security comes from the inner layer, not this TLS.

use boring::ssl::{SslConnector, SslMethod, SslVerifyMode};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_boring::SslStream;

/// Chrome 120-class cipher suites for TLS 1.2 (TLS 1.3 suites are fixed by
/// BoringSSL and already match Chrome). Order matters for JA3.
const CHROME_CIPHER_LIST: &str = concat!(
    "ECDHE-ECDSA-AES128-GCM-SHA256:",
    "ECDHE-RSA-AES128-GCM-SHA256:",
    "ECDHE-ECDSA-AES256-GCM-SHA384:",
    "ECDHE-RSA-AES256-GCM-SHA384:",
    "ECDHE-ECDSA-CHACHA20-POLY1305:",
    "ECDHE-RSA-CHACHA20-POLY1305:",
    "ECDHE-RSA-AES128-SHA:",
    "ECDHE-RSA-AES256-SHA:",
    "AES128-GCM-SHA256:",
    "AES256-GCM-SHA384:",
    "AES128-SHA:",
    "AES256-SHA"
);

/// Build the ALPN wire format (`len`-prefixed protocol list) BoringSSL wants.
fn alpn_wire(alpn: &[&[u8]]) -> Vec<u8> {
    let mut wire = Vec::new();
    for p in alpn {
        wire.push(p.len() as u8);
        wire.extend_from_slice(p);
    }
    wire
}

/// Perform a Chrome-fingerprinted TLS handshake over `stream` (typically a
/// `FragmentStream` so the ClientHello is also TCP-fragmented). `sni` is the
/// SNI/Host to present; `alpn` is the protocol list (e.g. `[b"h2",
/// b"http/1.1"]`). Returns an `SslStream` that tokio-tungstenite can drive.
pub async fn connect_chrome<S>(stream: S, sni: &str, alpn: &[&[u8]]) -> Result<SslStream<S>, String>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut builder = SslConnector::builder(SslMethod::tls())
        .map_err(|e| format!("boring builder init failed: {}", e))?;

    // TLS is camouflage only; the phantom layer authenticates separately.
    builder.set_verify(SslVerifyMode::NONE);

    // GREASE is the single biggest Chrome JA3 tell — BoringSSL implements it
    // the same way Chrome does.
    builder.set_grease_enabled(true);

    // Chrome cipher ordering for TLS 1.2.
    builder
        .set_cipher_list(CHROME_CIPHER_LIST)
        .map_err(|e| format!("boring set_cipher_list failed: {}", e))?;

    // ALPN exactly as the caller wants (Chrome sends h2,http/1.1).
    let wire = alpn_wire(alpn);
    builder
        .set_alpn_protos(&wire)
        .map_err(|e| format!("boring set_alpn_protos failed: {}", e))?;

    let connector = builder.build();
    let mut config = connector
        .configure()
        .map_err(|e| format!("boring configure failed: {}", e))?;

    // Send SNI, but do not enforce hostname verification (verify is NONE).
    config.set_verify_hostname(false);
    config.set_use_server_name_indication(true);

    tokio_boring::connect(config, sni, stream)
        .await
        .map_err(|e| format!("Chrome TLS handshake failed (SNI {}): {}", sni, e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpStream;

    /// Proves the Chrome-JA3 path completes a real TLS handshake to the
    /// known-good trojan endpoint and negotiates ALPN like a browser. From a
    /// non-censored network this must succeed; the point is to validate the
    /// boring wiring, not censorship behaviour.
    #[tokio::test]
    async fn chrome_handshake_to_real_endpoint() {
        let tcp = TcpStream::connect("172.64.152.23:443")
            .await
            .expect("tcp connect");
        let alpn: Vec<&[u8]> = vec![b"h2", b"http/1.1"];
        let tls = connect_chrome(tcp, "www.creationlong.org", &alpn)
            .await
            .expect("chrome tls handshake");
        let negotiated = tls
            .ssl()
            .selected_alpn_protocol()
            .map(|p| String::from_utf8_lossy(p).into_owned())
            .unwrap_or_default();
        assert!(
            negotiated == "h2" || negotiated == "http/1.1",
            "expected browser ALPN, got {:?}",
            negotiated
        );
    }
}
