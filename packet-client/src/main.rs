use clap::Parser;

#[derive(Parser)]
#[command(name = "phantom-client")]
#[command(about = "Phantom Tunnel Client — SOCKS5 proxy with covert tunnel (CDN bypass support)")]
struct Cli {
    /// Server URL (e.g. http://piano-lessons.site or http://35.222.22.49:80)
    #[arg(short = 'S', long)]
    server: String,

    /// Shared secret (must match server)
    #[arg(short, long, env = "PHANTOM_SECRET")]
    secret: String,

    /// Local SOCKS5 listen address
    #[arg(short, long, default_value = "127.0.0.1:1080")]
    listen: String,

    /// Transport mode: ws, http, or auto (default: auto)
    /// "ws" = WebSocket (CDN-compatible, recommended for Iran)
    /// "http" = HTTP POST polling (fallback)
    /// "auto" = try WebSocket first, fall back to HTTP
    #[arg(short, long, default_value = "auto")]
    transport: String,

    /// CDN edge IP:port to connect to instead of the server URL's host.
    /// Use this in Iran to connect through ArvanCloud's domestic edge nodes.
    /// Example: --cdn-edge 185.143.234.235:80
    #[arg(long)]
    cdn_edge: Option<String>,

    /// Custom Host header for CDN mode.
    /// When connecting through a CDN edge, this tells the CDN which
    /// origin domain to forward to.
    /// Example: --host piano-lessons.site
    #[arg(long)]
    host: Option<String>,

    /// Override the TLS SNI sent during the HTTPS/WSS handshake.
    /// Useful when the TCP destination, HTTP Host header, and visible SNI
    /// must be different.
    #[arg(long)]
    sni: Option<String>,

    /// Enable TLS ClientHello fragmentation.
    /// Splits the TLS handshake across multiple TCP segments to prevent
    /// DPI from reading the SNI field. Only useful for HTTPS connections.
    #[arg(long)]
    fragment: bool,

    /// Fragment chunk size in bytes (default: 40).
    /// The SNI field typically starts at byte 40-60 in the ClientHello.
    #[arg(long, default_value = "40")]
    fragment_size: usize,

    /// Disable traffic padding.
    /// By default, messages are padded to fixed block sizes to prevent
    /// DPI from fingerprinting the tunnel by message size patterns.
    #[arg(long)]
    no_padding: bool,
}

fn parse_transport(s: &str) -> phantom_client::TransportMode {
    match s.to_lowercase().as_str() {
        "ws" | "websocket" => phantom_client::TransportMode::WebSocket,
        "http" | "poll" | "polling" => phantom_client::TransportMode::Http,
        _ => phantom_client::TransportMode::Auto,
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();

    let config = phantom_client::ClientConfig {
        server_url: cli.server,
        secret: cli.secret,
        listen: cli.listen,
        transport: parse_transport(&cli.transport),
        cdn_edge: cli.cdn_edge,
        host_override: cli.host,
        fragment: cli.fragment,
        fragment_size: cli.fragment_size,
        padding: !cli.no_padding,
        sni_override: cli.sni,
        ..Default::default()
    };

    phantom_client::start_client_with_config(config).await;
}
