use clap::Parser;

#[derive(Parser)]
#[command(name = "phantom-stealth-client")]
#[command(about = "Phantom Tunnel Client — advanced HTTPS browser-like bypass mode")]
struct Cli {
    /// HTTPS server URL (for example https://piano-lessons.site)
    #[arg(short = 'S', long)]
    server: String,

    /// Shared secret (must match server)
    #[arg(short, long, env = "PHANTOM_SECRET")]
    secret: String,

    /// Local SOCKS5 listen address
    #[arg(short, long, default_value = "127.0.0.1:1080")]
    listen: String,

    /// CDN edge host/IP:port to connect to instead of the server URL host.
    #[arg(long)]
    cdn_edge: Option<String>,

    /// Custom Host header for the fronted HTTPS request.
    #[arg(long)]
    host: Option<String>,

    /// Override the TLS SNI sent in ClientHello.
    #[arg(long)]
    sni: Option<String>,

    /// Enable TLS ClientHello fragmentation.
    #[arg(long)]
    fragment: bool,

    /// Fragment chunk size in bytes.
    #[arg(long, default_value = "40")]
    fragment_size: usize,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();

    let config = phantom_client::ClientConfig {
        server_url: cli.server,
        secret: cli.secret,
        listen: cli.listen,
        transport: phantom_client::TransportMode::Stealth,
        cdn_edge: cli.cdn_edge,
        host_override: cli.host,
        sni_override: cli.sni,
        fragment: cli.fragment,
        fragment_size: cli.fragment_size,
        padding: true,
        tls_profile: phantom_client::TlsProfile::BrowserLike,
        ..Default::default()
    };

    phantom_client::start_client_with_config(config).await;
}
