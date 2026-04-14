use clap::Parser;

#[derive(Parser)]
#[command(name = "phantom-client")]
#[command(about = "Phantom Tunnel Client — local SOCKS5 proxy with covert HTTP tunnel")]
struct Cli {
    /// Server URL (e.g. https://piano-lessons.site or http://1.2.3.4:80)
    #[arg(short = 'S', long)]
    server: String,

    /// Shared secret (must match server)
    #[arg(short, long, env = "PHANTOM_SECRET")]
    secret: String,

    /// Local SOCKS5 listen address
    #[arg(short, long, default_value = "127.0.0.1:1080")]
    listen: String,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();
    
    phantom_client::start_client(cli.server, cli.secret, cli.listen).await;
}
