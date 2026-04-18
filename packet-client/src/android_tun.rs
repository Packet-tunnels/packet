use lazy_static::lazy_static;
use std::sync::Mutex;
use std::thread::{self, JoinHandle};
use tracing::{error, info};

const DEFAULT_TUN_IPV4: &str = "172.19.0.1";
const DEFAULT_MAP_DNS_IPV4: &str = "198.18.0.2";
const DEFAULT_MAP_DNS_NETWORK: &str = "240.0.0.0";
const DEFAULT_MAP_DNS_NETMASK: &str = "240.0.0.0";

lazy_static! {
    static ref ANDROID_TUN_WORKER: Mutex<Option<JoinHandle<()>>> = Mutex::new(None);
}

fn reap_finished_worker(worker: &mut Option<JoinHandle<()>>) {
    let is_finished = worker
        .as_ref()
        .map(|handle| handle.is_finished())
        .unwrap_or(false);

    if is_finished {
        if let Some(handle) = worker.take() {
            let _ = handle.join();
        }
    }
}

fn build_android_tun2socks_config(
    socks_addr: &str,
    socks_port: u16,
    mtu: i32,
    dns_addr: &str,
) -> String {
    format!(
        "tunnel:\n  mtu: {mtu}\n  ipv4: '{tun_ipv4}'\n\
         socks5:\n  address: '{socks_addr}'\n  port: {socks_port}\n  udp: 'udp'\n\
         mapdns:\n  address: '{dns_addr}'\n  port: 53\n  network: '{map_dns_network}'\n  netmask: '{map_dns_netmask}'\n\
         misc:\n  log-file: stderr\n  log-level: warn\n",
        mtu = mtu.max(1280),
        tun_ipv4 = DEFAULT_TUN_IPV4,
        socks_addr = socks_addr,
        socks_port = socks_port,
        dns_addr = dns_addr,
        map_dns_network = DEFAULT_MAP_DNS_NETWORK,
        map_dns_netmask = DEFAULT_MAP_DNS_NETMASK,
    )
}

pub fn start_android_tun_bridge(
    tun_fd: i32,
    socks_addr: &str,
    socks_port: u16,
    mtu: i32,
    dns_addr: Option<&str>,
) -> Result<(), String> {
    let mut worker = ANDROID_TUN_WORKER.lock().unwrap();
    reap_finished_worker(&mut worker);

    if worker.is_some() {
        return Err("Android tun bridge is already running".to_string());
    }

    let dup_fd = unsafe { libc::dup(tun_fd) };
    if dup_fd < 0 {
        return Err(format!(
            "Failed to duplicate Android tun fd {}: {}",
            tun_fd,
            std::io::Error::last_os_error()
        ));
    }

    let socks_addr = socks_addr.to_string();
    let dns_addr = dns_addr.unwrap_or(DEFAULT_MAP_DNS_IPV4).to_string();
    let config = build_android_tun2socks_config(&socks_addr, socks_port, mtu, &dns_addr);

    *worker = Some(thread::spawn(move || {
        info!(
            "[PHANTOM] Android tun bridge starting: tun_fd={} socks5={}:{} mtu={} dns={}",
            dup_fd,
            socks_addr,
            socks_port,
            mtu.max(1280),
            dns_addr,
        );

        match tun2socks::main_from_str(&config, dup_fd) {
            Ok(()) => info!("[PHANTOM] Android tun bridge stopped"),
            Err(code) => error!(
                "[PHANTOM] Android tun bridge exited with error code {}",
                code
            ),
        }
    }));

    Ok(())
}

pub fn stop_android_tun_bridge() {
    tun2socks::quit();

    let handle = {
        let mut worker = ANDROID_TUN_WORKER.lock().unwrap();
        reap_finished_worker(&mut worker);
        worker.take()
    };

    if let Some(handle) = handle {
        thread::spawn(move || {
            let _ = handle.join();
        });
    }
}
