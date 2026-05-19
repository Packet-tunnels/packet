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

    /// Disable TLS ClientHello fragmentation.
    /// Fragmentation is ON by default (v2rayNG `tlshello` preset: 5 writes
    /// of 100-150 bytes each, 10-20 ms apart). Pass --no-fragment to turn
    /// it off when connecting to non-DPI'd endpoints for diagnostics.
    #[arg(long)]
    no_fragment: bool,

    /// Hint for the lower bound of the fragment chunk size range (bytes).
    /// Default 0 → use the v2rayNG-style [100, 150] range as-is. Values
    /// inside [100, 150) shift the low edge up; other values are ignored.
    #[arg(long, default_value = "0")]
    fragment_size: usize,

    /// TLS profile: default or browser-like.
    /// "browser-like" advertises ALPN h2,http/1.1 and sends browser-ish HTTP headers.
    #[arg(long, default_value = "default")]
    tls_profile: String,

    /// Shortcut for --transport stealth --tls-profile browser-like.
    #[arg(long)]
    stealth: bool,

    /// Disable traffic padding.
    /// By default, messages are padded to fixed block sizes to prevent
    /// DPI from fingerprinting the tunnel by message size patterns.
    #[arg(long)]
    no_padding: bool,

    /// Number of parallel transport lanes. Each lane is an independent
    /// WS+TLS tunnel with its own SNI/ALPN/fragmentation fingerprint.
    /// Default 5 replicates stacked-tunnel parallelism.
    /// effect that bypasses Iran's per-flow DPI. Set to 1 to disable.
    #[arg(long, default_value = "5")]
    lanes: usize,

    /// Pin every lane's SNI to the operator-supplied --sni. By default lanes
    /// 1..N rotate through plausible fronting SNIs (cdnjs, fonts.gstatic,
    /// discord etc) for fingerprint diversity. Use this when your CDN
    /// requires exact-match SNI for routing.
    #[arg(long)]
    pin_sni: bool,

    /// Disable decoy cover traffic. By default the client periodically
    /// opens real TLS connections to popular Iranian sites (Aparat,
    /// Digikala, Snapp, Blu, Divar...) so the device looks like a normal
    /// user browsing instead of a pure tunneling appliance.
    #[arg(long)]
    no_decoy: bool,

    /// Number of concurrent decoy workers when decoy traffic is on.
    #[arg(long, default_value = "2")]
    decoy_workers: usize,

    /// Pre-shared obfuscation "knock" for --transport obfs. Must match the
    /// server's --obfs-key. Low-entropy is fine — confidentiality is the
    /// inner frame crypto. With obfs, pass the directly-reachable foreign
    /// IP:port via --cdn-edge (NOT a TLS-terminating CDN).
    #[arg(long, env = "PHANTOM_OBFS_KEY")]
    obfs_key: Option<String>,

    /// Optional first-hop proxy for Obfs, for example
    /// socks5://127.0.0.1:10808 or http://user:pass@bridge:8080.
    #[arg(long, env = "PHANTOM_UPSTREAM_PROXY")]
    upstream_proxy: Option<String>,
}

fn parse_transport(s: &str) -> phantom_client::TransportMode {
    match s.to_lowercase().as_str() {
        "ws" | "websocket" => phantom_client::TransportMode::WebSocket,
        "http" | "poll" | "polling" => phantom_client::TransportMode::Http,
        "stealth" | "browser" | "browser-like" | "browser_like" => {
            phantom_client::TransportMode::Stealth
        }
        "obfs" | "ossh" | "raw" => phantom_client::TransportMode::Obfs,
        _ => phantom_client::TransportMode::Auto,
    }
}

fn parse_tls_profile(s: &str) -> phantom_client::TlsProfile {
    match s.to_lowercase().as_str() {
        "browser" | "browser-like" | "browser_like" | "chrome" => {
            phantom_client::TlsProfile::BrowserLike
        }
        _ => phantom_client::TlsProfile::Default,
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();

    let transport = if cli.stealth {
        phantom_client::TransportMode::Stealth
    } else {
        parse_transport(&cli.transport)
    };
    let tls_profile = if cli.stealth || matches!(transport, phantom_client::TransportMode::Stealth)
    {
        phantom_client::TlsProfile::BrowserLike
    } else {
        parse_tls_profile(&cli.tls_profile)
    };

    let config = phantom_client::ClientConfig {
        server_url: cli.server,
        secret: cli.secret,
        listen: cli.listen,
        transport,
        cdn_edge: cli.cdn_edge,
        host_override: cli.host,
        fragment: !cli.no_fragment,
        fragment_size: cli.fragment_size,
        padding: !cli.no_padding,
        sni_override: cli.sni,
        tls_profile,
        multi_lane_count: cli.lanes,
        multi_lane_pin_sni: cli.pin_sni,
        decoy_traffic: !cli.no_decoy,
        decoy_workers: cli.decoy_workers,
        obfs_key: cli.obfs_key,
        upstream_proxy: cli.upstream_proxy,
        ..Default::default()
    };

    phantom_client::start_client_with_config(config).await;
}
