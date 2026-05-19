use clap::Parser;
use tokio::sync::watch;

#[derive(Parser)]
#[command(name = "phantom-layered-carrier")]
#[command(about = "Packet DirectSock: local mixed HTTP/SOCKS proxy -> Trojan TCP/WS TLS")]
struct Cli {
    /// Trojan WS TLS URI, for example:
    /// trojan://password@edge.example:443?security=tls&type=tcp&fp=chrome&sni=front.example
    #[arg(long)]
    trojan_uri: String,

    /// Local mixed HTTP/SOCKS proxy listen address for apps, PAC, or tun2socks.
    #[arg(short, long, default_value = "127.0.0.1:10808")]
    listen: String,

    /// Disable tlshello fragmentation on the DirectSock TLS handshake.
    ///
    /// Fragmentation is on by default to match the Psiphon/v2rayNG-style
    /// DirectSock profile. If an edge rejects the fragmented handshake, the core
    /// retries once without fragmentation.
    #[arg(long)]
    no_fragment: bool,

    /// Fragment size hint. Values inside 100-150 keep v2rayNG-style randomized tlshello chunks.
    #[arg(long, default_value = "100")]
    fragment_size: usize,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();

    let mut config =
        match phantom_client::trojan_carrier::TrojanCarrierConfig::from_uri(&cli.trojan_uri) {
            Ok(config) => config,
            Err(error) => {
                eprintln!("invalid DirectSock config: {}", error);
                std::process::exit(2);
            }
        };
    config.fragment_tls_hello = !cli.no_fragment;
    config.fragment_size_hint = cli.fragment_size;

    let std_listener = match phantom_client::bind_socks_listener(&cli.listen) {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("failed to bind {}: {}", cli.listen, error);
            std::process::exit(2);
        }
    };
    let (_shutdown_tx, shutdown_rx) = watch::channel(false);
    phantom_client::trojan_carrier::run_carrier_proxy(config, std_listener, shutdown_rx).await;
}
