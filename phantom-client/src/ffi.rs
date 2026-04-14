use std::ffi::CStr;
use std::os::raw::c_char;
use std::thread;
use tokio::runtime::Runtime;
use crate::{start_client, start_client_with_config, ClientConfig, TransportMode};

// ─── C API (iOS & General FFI) ─────────────────────────────────

/// Start Phantom Tunnel with basic configuration (backward compatible).
#[no_mangle]
pub extern "C" fn phantom_start(
    server_url: *const c_char,
    secret: *const c_char,
    listen_port: u16,
) -> i32 {
    let url = unsafe {
        if server_url.is_null() { return -1; }
        CStr::from_ptr(server_url).to_string_lossy().into_owned()
    };

    let sec = unsafe {
        if secret.is_null() { return -1; }
        CStr::from_ptr(secret).to_string_lossy().into_owned()
    };

    let listen_addr = format!("127.0.0.1:{}", listen_port);

    // Spawn the async runtime in a new background thread
    thread::spawn(move || {
        let rt = Runtime::new().expect("Failed to create tokio runtime");
        rt.block_on(async {
            start_client(url, sec, listen_addr).await;
        });
    });

    0 // Success
}

/// Start Phantom Tunnel with CDN bypass configuration.
///
/// Parameters:
/// - server_url: Server URL (e.g., "http://piano-lessons.site")
/// - secret: Shared secret
/// - listen_port: Local SOCKS5 port (e.g., 1080)
/// - cdn_edge: CDN edge IP:port (nullable, e.g., "185.143.234.235:80")
/// - host_override: Custom Host header (nullable, e.g., "piano-lessons.site")
/// - transport_mode: 0=auto, 1=websocket, 2=http
///
/// Returns 0 on success, -1 on error.
#[no_mangle]
pub extern "C" fn phantom_start_cdn(
    server_url: *const c_char,
    secret: *const c_char,
    listen_port: u16,
    cdn_edge: *const c_char,
    host_override: *const c_char,
    transport_mode: i32,
) -> i32 {
    let url = unsafe {
        if server_url.is_null() { return -1; }
        CStr::from_ptr(server_url).to_string_lossy().into_owned()
    };

    let sec = unsafe {
        if secret.is_null() { return -1; }
        CStr::from_ptr(secret).to_string_lossy().into_owned()
    };

    let edge = unsafe {
        if cdn_edge.is_null() {
            None
        } else {
            Some(CStr::from_ptr(cdn_edge).to_string_lossy().into_owned())
        }
    };

    let host = unsafe {
        if host_override.is_null() {
            None
        } else {
            Some(CStr::from_ptr(host_override).to_string_lossy().into_owned())
        }
    };

    let transport = match transport_mode {
        1 => TransportMode::WebSocket,
        2 => TransportMode::Http,
        _ => TransportMode::Auto,
    };

    let config = ClientConfig {
        server_url: url,
        secret: sec,
        listen: format!("127.0.0.1:{}", listen_port),
        transport,
        cdn_edge: edge,
        host_override: host,
        fragment: false,
        fragment_size: 40,
        padding: true,
    };

    thread::spawn(move || {
        let rt = Runtime::new().expect("Failed to create tokio runtime");
        rt.block_on(async {
            start_client_with_config(config).await;
        });
    });

    0 // Success
}

// ─── JNI (Android) ─────────────────────────────────────────────
#[cfg(target_os = "android")]
#[allow(non_snake_case)]
pub mod android {
    use super::*;
    use jni::JNIEnv;
    use jni::objects::{JClass, JString};
    use jni::sys::jint;

    /// Basic start (backward compatible)
    #[no_mangle]
    pub extern "system" fn Java_com_resolo_phantom_PhantomTunnel_startClient(
        mut env: JNIEnv,
        _class: JClass,
        server_url: JString,
        secret: JString,
        listen_port: jint,
    ) {
        let url: String = env.get_string(&server_url).expect("Couldn't get url").into();
        let sec: String = env.get_string(&secret).expect("Couldn't get secret").into();
        let listen_addr = format!("127.0.0.1:{}", listen_port);

        thread::spawn(move || {
            let rt = Runtime::new().expect("Failed to create tokio runtime");
            rt.block_on(async {
                start_client(url, sec, listen_addr).await;
            });
        });
    }

    /// CDN bypass start with full configuration
    #[no_mangle]
    pub extern "system" fn Java_com_resolo_phantom_PhantomTunnel_startClientCdn(
        mut env: JNIEnv,
        _class: JClass,
        server_url: JString,
        secret: JString,
        listen_port: jint,
        cdn_edge: JString,
        host_override: JString,
        transport_mode: jint,
    ) {
        let url: String = env.get_string(&server_url).expect("url").into();
        let sec: String = env.get_string(&secret).expect("secret").into();

        let edge_str: String = env.get_string(&cdn_edge).unwrap_or_default().into();
        let host_str: String = env.get_string(&host_override).unwrap_or_default().into();

        let edge = if edge_str.is_empty() { None } else { Some(edge_str) };
        let host = if host_str.is_empty() { None } else { Some(host_str) };

        let transport = match transport_mode {
            1 => TransportMode::WebSocket,
            2 => TransportMode::Http,
            _ => TransportMode::Auto,
        };

        let config = ClientConfig {
            server_url: url,
            secret: sec,
            listen: format!("127.0.0.1:{}", listen_port),
            transport,
            cdn_edge: edge,
            host_override: host,
            fragment: false,
            fragment_size: 40,
            padding: true,
        };

        thread::spawn(move || {
            let rt = Runtime::new().expect("Failed to create tokio runtime");
            rt.block_on(async {
                start_client_with_config(config).await;
            });
        });
    }
}
