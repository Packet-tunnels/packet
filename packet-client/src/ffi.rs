use crate::{
    bind_socks_listener, mesh, start_client_with_listener, trojan_carrier, ClientConfig,
    TlsProfile, TransportMode,
};
use phantom_proto::{MeshBootstrapConfig, PacketPeerDescriptor};
use serde::Deserialize;
use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::sync::Mutex;
use std::thread;
use tokio::runtime::Runtime;
use tokio::sync::watch;
use tracing_subscriber::fmt::MakeWriter;

#[cfg(target_os = "android")]
use jni::errors::Result as JniResult;
#[cfg(target_os = "android")]
use jni::objects::{Global, JObject, JValue};
#[cfg(target_os = "android")]
use jni::JavaVM;

// ─── Logging Integration ───────────────────────────────────────

lazy_static::lazy_static! {
    static ref LOG_CALLBACK: Mutex<Option<extern "C" fn(*const c_char)>> = Mutex::new(None);
}

struct ActiveClient {
    shutdown_tx: watch::Sender<bool>,
    handle: thread::JoinHandle<()>,
}

struct ActiveCarrier {
    shutdown_tx: watch::Sender<bool>,
    handle: thread::JoinHandle<()>,
}

#[derive(Debug, Deserialize)]
struct MeshStartRequest {
    server_url: String,
    ticket: String,
    #[serde(default)]
    listen_port: Option<u16>,
    #[serde(default)]
    transport_mode: Option<String>,
    #[serde(default)]
    cdn_edge: Option<String>,
    #[serde(default)]
    host_override: Option<String>,
    #[serde(default)]
    sni_override: Option<String>,
    #[serde(default)]
    fragment: bool,
    #[serde(default)]
    fragment_size: Option<usize>,
    #[serde(default)]
    bootstrap: Option<MeshBootstrapConfig>,
    #[serde(default)]
    tls_profile: Option<String>,
    #[serde(default)]
    obfs_key: Option<String>,
}

lazy_static::lazy_static! {
    static ref ACTIVE_CLIENT: Mutex<Option<ActiveClient>> = Mutex::new(None);
    static ref ACTIVE_CARRIER: Mutex<Option<ActiveCarrier>> = Mutex::new(None);
}

#[cfg(target_os = "android")]
struct AndroidLogCallback {
    vm: JavaVM,
    callback: Global<JObject<'static>>,
}

#[cfg(target_os = "android")]
lazy_static::lazy_static! {
    static ref ANDROID_LOG_CALLBACK: Mutex<Option<AndroidLogCallback>> = Mutex::new(None);
}

#[cfg(target_os = "android")]
fn dispatch_android_log(message: &str) {
    let guard = ANDROID_LOG_CALLBACK.lock().unwrap();
    let Some(callback) = guard.as_ref() else {
        print!("{}", message);
        return;
    };

    let _ = callback.vm.attach_current_thread(|env| -> JniResult<()> {
        let text = env.new_string(message)?;
        let text_obj = JObject::from(text);
        let log_arg = JValue::Object(&text_obj);
        env.call_method(
            callback.callback.as_obj(),
            jni::jni_str!("onLog"),
            jni::jni_sig!("(Ljava/lang/String;)V"),
            &[log_arg],
        )?;
        Ok(())
    });
}

#[no_mangle]
pub extern "C" fn phantom_set_log_callback(cb: extern "C" fn(*const c_char)) {
    *LOG_CALLBACK.lock().unwrap() = Some(cb);
}

struct NativeLogWriter;

impl std::io::Write for NativeLogWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let msg = String::from_utf8_lossy(buf);

        #[cfg(target_os = "android")]
        {
            dispatch_android_log(msg.as_ref());
            Ok(buf.len())
        }

        #[cfg(not(target_os = "android"))]
        {
            let c_str =
                CString::new(msg.as_ref()).unwrap_or_else(|_| CString::new("log error").unwrap());

            if let Some(cb) = *LOG_CALLBACK.lock().unwrap() {
                cb(c_str.as_ptr());
            } else {
                // Fallback to stdout if no callback set
                print!("{}", msg);
            }
            Ok(buf.len())
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for NativeLogWriter {
    type Writer = NativeLogWriter;
    fn make_writer(&'a self) -> Self::Writer {
        NativeLogWriter
    }
}

fn init_logging() {
    let _ = tracing_subscriber::fmt()
        .with_target(false)
        .with_thread_ids(false)
        .with_file(false)
        .with_line_number(false)
        .with_ansi(false)
        .with_writer(NativeLogWriter)
        .try_init();
}

fn bound_listen_addr(listener: &std::net::TcpListener) -> Result<String, String> {
    let local_addr = listener
        .local_addr()
        .map_err(|e| format!("Failed to read bound local address: {}", e))?;
    Ok(format!("127.0.0.1:{}", local_addr.port()))
}

fn bind_requested_or_auto_listener(
    requested_port: u16,
) -> Result<(std::net::TcpListener, String, u16), String> {
    let requested_addr = format!("127.0.0.1:{}", requested_port);

    match bind_socks_listener(&requested_addr) {
        Ok(listener) => {
            let listen_addr = bound_listen_addr(&listener)?;
            let actual_port = listener
                .local_addr()
                .map_err(|e| format!("Failed to read bound local address: {}", e))?
                .port();

            if requested_port == 0 {
                tracing::info!("[PHANTOM] Auto-selected local SOCKS5 port {}", actual_port);
            }

            Ok((listener, listen_addr, actual_port))
        }
        Err(request_error) if requested_port != 0 => {
            tracing::warn!(
                "[PHANTOM] Requested local SOCKS5 port {} is busy, retrying with auto port",
                requested_port
            );

            let listener = bind_socks_listener("127.0.0.1:0").map_err(|fallback_error| {
                format!(
                    "{}; fallback to automatic local port failed: {}",
                    request_error, fallback_error
                )
            })?;

            let actual_port = listener
                .local_addr()
                .map_err(|e| format!("Failed to read bound local address: {}", e))?
                .port();
            let listen_addr = format!("127.0.0.1:{}", actual_port);

            tracing::info!(
                "[PHANTOM] Falling back from requested local SOCKS5 port {} to {}",
                requested_port,
                actual_port
            );

            Ok((listener, listen_addr, actual_port))
        }
        Err(error) => Err(error),
    }
}

fn spawn_client_with_bound_listener(config: ClientConfig, listener: std::net::TcpListener) -> i32 {
    let actual_port = listener
        .local_addr()
        .map(|address| address.port())
        .unwrap_or_default();

    stop_active_client();

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let handle = thread::spawn(move || {
        let rt = Runtime::new().expect("Failed to create tokio runtime");
        rt.block_on(async {
            start_client_with_listener(config, listener, shutdown_rx).await;
        });
    });

    *ACTIVE_CLIENT.lock().unwrap() = Some(ActiveClient {
        shutdown_tx,
        handle,
    });

    i32::from(actual_port)
}

fn stop_active_client() {
    let active_client = ACTIVE_CLIENT.lock().unwrap().take();
    if let Some(active_client) = active_client {
        let _ = active_client.shutdown_tx.send(true);
        let _ = active_client.handle.join();
        crate::clear_runtime_stats();
    }
}

fn spawn_carrier_with_bound_listener(
    config: trojan_carrier::TrojanCarrierConfig,
    listener: std::net::TcpListener,
) -> i32 {
    let actual_port = listener
        .local_addr()
        .map(|address| address.port())
        .unwrap_or_default();

    stop_active_carrier();

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let handle = thread::spawn(move || {
        let rt = Runtime::new().expect("Failed to create tokio runtime");
        rt.block_on(async {
            trojan_carrier::run_carrier_proxy(config, listener, shutdown_rx).await;
        });
    });

    *ACTIVE_CARRIER.lock().unwrap() = Some(ActiveCarrier {
        shutdown_tx,
        handle,
    });

    i32::from(actual_port)
}

fn stop_active_carrier() {
    let active_carrier = ACTIVE_CARRIER.lock().unwrap().take();
    if let Some(active_carrier) = active_carrier {
        let _ = active_carrier.shutdown_tx.send(true);
        let _ = active_carrier.handle.join();
    }
}

#[no_mangle]
pub extern "C" fn phantom_emit_test_output() {
    init_logging();
    tracing::info!("[PHANTOM] iOS test bridge is active");
    tracing::info!("[PHANTOM] Rust log callback delivered output to SwiftUI");
}

// ─── C API (iOS & General FFI) ─────────────────────────────────

/// Returns a JSON string containing the current tunnel stats, or null if unavailable.
/// The caller MUST free the returned string using `phantom_free_string`.
#[no_mangle]
pub extern "C" fn phantom_copy_stats_json() -> *mut c_char {
    let json = crate::runtime_stats_json().unwrap_or_else(|| "{}".to_string());
    CString::new(json)
        .unwrap_or_else(|_| CString::new("{}").unwrap())
        .into_raw()
}

#[no_mangle]
pub extern "C" fn phantom_copy_mesh_stats_json() -> *mut c_char {
    let json = mesh::stats_json().unwrap_or_else(|| "{}".to_string());
    CString::new(json)
        .unwrap_or_else(|_| CString::new("{}").unwrap())
        .into_raw()
}

/// Runs the connection diagnostic against the given trojan:// URI and
/// returns a human-readable report. The caller MUST free the returned
/// string with `phantom_free_string`. Blocks until the probe completes
/// (a few seconds). Intended to be called off the UI thread.
#[no_mangle]
pub extern "C" fn phantom_run_diagnostic(trojan_uri: *const c_char) -> *mut c_char {
    init_logging();
    let uri = unsafe {
        if trojan_uri.is_null() {
            return CString::new("ERROR: null uri").unwrap().into_raw();
        }
        CStr::from_ptr(trojan_uri).to_string_lossy().into_owned()
    };
    let report = match Runtime::new() {
        Ok(rt) => rt.block_on(async { crate::diagnostic::run_diagnostic(&uri).await }),
        Err(e) => format!("ERROR: could not start runtime: {}", e),
    };
    CString::new(report)
        .unwrap_or_else(|_| CString::new("ERROR: report contained NUL").unwrap())
        .into_raw()
}

/// Frees a string previously allocated by `phantom_copy_stats_json`.
#[no_mangle]
pub extern "C" fn phantom_free_string(s: *mut c_char) {
    if s.is_null() {
        return;
    }
    unsafe {
        let _ = CString::from_raw(s);
    }
}

#[no_mangle]
pub extern "C" fn phantom_stop_client() {
    init_logging();
    stop_active_client();
}

#[no_mangle]
pub extern "C" fn phantom_stop_layered_carrier() {
    init_logging();
    stop_active_carrier();
}

#[no_mangle]
pub extern "C" fn phantom_start_layered_carrier(
    trojan_uri: *const c_char,
    listen_port: u16,
) -> i32 {
    phantom_start_layered_carrier_full(trojan_uri, listen_port, 1, 100)
}

#[no_mangle]
pub extern "C" fn phantom_start_layered_carrier_full(
    trojan_uri: *const c_char,
    listen_port: u16,
    fragment_enabled: i32,
    fragment_size: u32,
) -> i32 {
    let uri = unsafe {
        if trojan_uri.is_null() {
            return -1;
        }
        CStr::from_ptr(trojan_uri).to_string_lossy().into_owned()
    };

    init_logging();
    let mut config = match trojan_carrier::TrojanCarrierConfig::from_uri(&uri) {
        Ok(config) => config,
        Err(error) => {
            tracing::error!("[carrier] ❌ {}", error);
            return -3;
        }
    };
    config.fragment_tls_hello = fragment_enabled != 0;
    config.fragment_size_hint = if fragment_size == 0 {
        100
    } else {
        fragment_size as usize
    };

    let (listener, _, actual_port) = match bind_requested_or_auto_listener(listen_port) {
        Ok(bound) => bound,
        Err(error) => {
            tracing::error!("[carrier] ❌ {}", error);
            return -2;
        }
    };

    tracing::info!(
        "[carrier] Starting DirectSock Trojan bridge on 127.0.0.1:{} fragment_tls_hello={} fragment_hint={}",
        actual_port,
        config.fragment_tls_hello,
        config.fragment_size_hint
    );
    crate::reset_carrier_runtime_stats(
        config.endpoint.host.clone(),
        Some(config.endpoint.connect_addr()),
        actual_port,
    );
    spawn_carrier_with_bound_listener(config, listener)
}

#[no_mangle]
pub extern "C" fn phantom_import_mesh_peers(peers_json: *const c_char) -> i32 {
    let payload = unsafe {
        if peers_json.is_null() {
            return -1;
        }
        CStr::from_ptr(peers_json).to_string_lossy().into_owned()
    };

    let peers: Vec<PacketPeerDescriptor> = match serde_json::from_str(&payload) {
        Ok(peers) => peers,
        Err(error) => {
            tracing::error!("[PHANTOM] ❌ Failed to parse mesh peers JSON: {}", error);
            return -2;
        }
    };

    mesh::import_peers(peers);
    0
}

/// Start Phantom Tunnel with basic configuration (backward compatible).
#[no_mangle]
pub extern "C" fn phantom_start(
    server_url: *const c_char,
    secret: *const c_char,
    listen_port: u16,
) -> i32 {
    let url = unsafe {
        if server_url.is_null() {
            return -1;
        }
        CStr::from_ptr(server_url).to_string_lossy().into_owned()
    };

    let sec = unsafe {
        if secret.is_null() {
            return -1;
        }
        CStr::from_ptr(secret).to_string_lossy().into_owned()
    };

    init_logging();
    let (listener, listen_addr, _) = match bind_requested_or_auto_listener(listen_port) {
        Ok(bound) => bound,
        Err(e) => {
            tracing::error!("[PHANTOM] ❌ {}", e);
            return -2;
        }
    };
    spawn_client_with_bound_listener(
        ClientConfig {
            server_url: url,
            secret: sec,
            listen: listen_addr,
            transport: TransportMode::Auto,
            ..Default::default()
        },
        listener,
    )
}

#[no_mangle]
pub extern "C" fn phantom_start_mesh(config_json: *const c_char, listen_port: u16) -> i32 {
    let payload = unsafe {
        if config_json.is_null() {
            return -1;
        }
        CStr::from_ptr(config_json).to_string_lossy().into_owned()
    };

    init_logging();
    match start_mesh_from_json(&payload, listen_port) {
        Ok(port) => port,
        Err(error) => {
            tracing::error!("[PHANTOM] ❌ {}", error);
            -2
        }
    }
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
        if server_url.is_null() {
            return -1;
        }
        CStr::from_ptr(server_url).to_string_lossy().into_owned()
    };

    let sec = unsafe {
        if secret.is_null() {
            return -1;
        }
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

    let transport = parse_transport_mode_value(transport_mode);

    init_logging();
    let (listener, listen_addr, _) = match bind_requested_or_auto_listener(listen_port) {
        Ok(bound) => bound,
        Err(e) => {
            tracing::error!("[PHANTOM] ❌ {}", e);
            return -2;
        }
    };

    let config = ClientConfig {
        server_url: url,
        secret: sec,
        listen: listen_addr,
        transport: transport.clone(),
        cdn_edge: edge,
        host_override: host,
        fragment: false, // <-- REVERTED: Fragmenting TLS to a WAF often triggers Slowloris protection
        fragment_size: 40,
        padding: true,
        sni_override: None,
        auth_ticket: None,
        mesh_bootstrap: None,
        tls_profile: tls_profile_for_transport(&transport),
        ..Default::default()
    };

    spawn_client_with_bound_listener(config, listener)
}

/// Full start for iOS/general FFI: CDN edge + host + SNI + fragmentation + TLS profile.
#[no_mangle]
pub extern "C" fn phantom_start_full(
    server_url: *const c_char,
    secret: *const c_char,
    listen_port: u16,
    cdn_edge: *const c_char,
    host_override: *const c_char,
    sni_override: *const c_char,
    transport_mode: i32,
    fragment_enabled: i32,
    fragment_size: u32,
    tls_profile: i32,
    obfs_key: *const c_char,
) -> i32 {
    let url = unsafe {
        if server_url.is_null() {
            return -1;
        }
        CStr::from_ptr(server_url).to_string_lossy().into_owned()
    };

    let sec = unsafe {
        if secret.is_null() {
            return -1;
        }
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

    let sni = unsafe {
        if sni_override.is_null() {
            None
        } else {
            Some(CStr::from_ptr(sni_override).to_string_lossy().into_owned())
        }
    };
    let obfs_key = unsafe {
        if obfs_key.is_null() {
            None
        } else {
            let value = CStr::from_ptr(obfs_key).to_string_lossy().into_owned();
            if value.trim().is_empty() {
                None
            } else {
                Some(value)
            }
        }
    };

    let transport = parse_transport_mode_value(transport_mode);
    let requested_tls_profile = parse_tls_profile_value(tls_profile);

    init_logging();
    let (listener, listen_addr, _) = match bind_requested_or_auto_listener(listen_port) {
        Ok(bound) => bound,
        Err(e) => {
            tracing::error!("[PHANTOM] ❌ {}", e);
            return -2;
        }
    };

    let config = ClientConfig {
        server_url: url,
        secret: sec,
        listen: listen_addr,
        transport: transport.clone(),
        cdn_edge: edge,
        host_override: host,
        sni_override: sni,
        fragment: fragment_enabled != 0,
        fragment_size: fragment_size as usize,
        padding: true,
        auth_ticket: None,
        mesh_bootstrap: None,
        tls_profile: requested_tls_profile.unwrap_or_else(|| tls_profile_for_transport(&transport)),
        obfs_key,
        ..Default::default()
    };

    spawn_client_with_bound_listener(config, listener)
}

// ─── JNI (Android) ─────────────────────────────────────────────
#[cfg(target_os = "android")]
#[allow(non_snake_case)]
pub mod android {
    use super::*;
    use jni::errors::LogErrorAndDefault;
    use jni::objects::{JClass, JObject};
    use jni::sys::{jboolean, jint, jstring};
    use jni::{objects::JString, EnvUnowned};

    #[no_mangle]
    pub extern "system" fn Java_com_resolo_phantom_PhantomTunnel_setLogCallback(
        mut env: EnvUnowned,
        _class: JClass,
        callback: JObject,
    ) {
        env.with_env(|env| -> JniResult<()> {
            let vm = env.get_java_vm()?;
            let callback = env.new_global_ref(callback)?;
            *ANDROID_LOG_CALLBACK.lock().unwrap() = Some(AndroidLogCallback { vm, callback });
            init_logging();
            Ok(())
        })
        .resolve::<LogErrorAndDefault>();
    }

    #[no_mangle]
    pub extern "system" fn Java_com_resolo_phantom_PhantomTunnel_emitTestOutput(
        _env: EnvUnowned,
        _class: JClass,
    ) {
        init_logging();
        tracing::info!("[PHANTOM] Android test bridge is active");
        tracing::info!("[PHANTOM] Rust log callback delivered output to Kotlin");
    }

    #[no_mangle]
    pub extern "system" fn Java_com_resolo_phantom_PhantomTunnel_copyStatsJson(
        mut env: EnvUnowned,
        _class: JClass,
    ) -> jstring {
        env.with_env(|env| -> JniResult<jstring> {
            let json = crate::runtime_stats_json().unwrap_or_else(|| "{}".to_string());
            let java_string = env.new_string(json)?;
            Ok(java_string.into_raw())
        })
        .resolve::<LogErrorAndDefault>()
    }

    #[no_mangle]
    pub extern "system" fn Java_com_resolo_phantom_PhantomTunnel_copyMeshStatsJson(
        mut env: EnvUnowned,
        _class: JClass,
    ) -> jstring {
        env.with_env(|env| -> JniResult<jstring> {
            let json = mesh::stats_json().unwrap_or_else(|| "{}".to_string());
            let java_string = env.new_string(json)?;
            Ok(java_string.into_raw())
        })
        .resolve::<LogErrorAndDefault>()
    }

    #[no_mangle]
    pub extern "system" fn Java_com_resolo_phantom_PhantomTunnel_runDiagnostic(
        mut env: EnvUnowned,
        _class: JClass,
        trojan_uri: JString,
    ) -> jstring {
        env.with_env(|env| -> JniResult<jstring> {
            let uri = trojan_uri.try_to_string(env)?;
            init_logging();
            let report = match Runtime::new() {
                Ok(rt) => rt.block_on(async { crate::diagnostic::run_diagnostic(&uri).await }),
                Err(e) => format!("ERROR: could not start runtime: {}", e),
            };
            let java_string = env.new_string(report)?;
            Ok(java_string.into_raw())
        })
        .resolve::<LogErrorAndDefault>()
    }

    #[no_mangle]
    pub extern "system" fn Java_com_resolo_phantom_PhantomTunnel_stopClient(
        _env: EnvUnowned,
        _class: JClass,
    ) {
        init_logging();
        stop_active_client();
    }

    #[no_mangle]
    pub extern "system" fn Java_com_resolo_phantom_PhantomTunnel_stopLayeredCarrier(
        _env: EnvUnowned,
        _class: JClass,
    ) {
        init_logging();
        stop_active_carrier();
    }

    #[no_mangle]
    pub extern "system" fn Java_com_resolo_phantom_PhantomTunnel_startLayeredCarrier(
        mut env: EnvUnowned,
        _class: JClass,
        trojan_uri: JString,
        listen_port: jint,
    ) -> jint {
        start_layered_carrier_android(&mut env, trojan_uri, listen_port, true, 100)
    }

    #[no_mangle]
    pub extern "system" fn Java_com_resolo_phantom_PhantomTunnel_startLayeredCarrierFull(
        mut env: EnvUnowned,
        _class: JClass,
        trojan_uri: JString,
        listen_port: jint,
        fragment_enabled: jboolean,
        fragment_size: jint,
    ) -> jint {
        start_layered_carrier_android(
            &mut env,
            trojan_uri,
            listen_port,
            fragment_enabled,
            fragment_size,
        )
    }

    fn start_layered_carrier_android(
        env: &mut EnvUnowned,
        trojan_uri: JString,
        listen_port: jint,
        fragment_enabled: bool,
        fragment_size: jint,
    ) -> jint {
        env.with_env(|env| -> JniResult<jint> {
            let uri = trojan_uri.try_to_string(env)?;
            init_logging();

            let mut config = match trojan_carrier::TrojanCarrierConfig::from_uri(&uri) {
                Ok(config) => config,
                Err(error) => {
                    tracing::error!("[carrier] ❌ {}", error);
                    return Ok(-3);
                }
            };
            config.fragment_tls_hello = fragment_enabled;
            config.fragment_size_hint = if fragment_size <= 0 {
                100
            } else {
                fragment_size as usize
            };

            let (listener, _, actual_port) =
                match bind_requested_or_auto_listener(listen_port as u16) {
                    Ok(bound) => bound,
                    Err(error) => {
                        tracing::error!("[carrier] ❌ {}", error);
                        return Ok(-2);
                    }
                };

            tracing::info!(
                "[carrier] Starting DirectSock Trojan bridge on 127.0.0.1:{} fragment_tls_hello={} fragment_hint={}",
                actual_port,
                config.fragment_tls_hello,
                config.fragment_size_hint
            );
            crate::reset_carrier_runtime_stats(
                config.endpoint.host.clone(),
                Some(config.endpoint.connect_addr()),
                actual_port,
            );
            Ok(spawn_carrier_with_bound_listener(config, listener) as jint)
        })
        .resolve::<LogErrorAndDefault>()
    }

    /// Basic start (backward compatible)
    #[no_mangle]
    pub extern "system" fn Java_com_resolo_phantom_PhantomTunnel_startClient(
        mut env: EnvUnowned,
        _class: JClass,
        server_url: JString,
        secret: JString,
        listen_port: jint,
    ) -> jint {
        env.with_env(|env| -> JniResult<jint> {
            let url = server_url.try_to_string(env)?;
            let sec = secret.try_to_string(env)?;
            init_logging();

            let (listener, listen_addr, _) =
                match bind_requested_or_auto_listener(listen_port as u16) {
                    Ok(bound) => bound,
                    Err(e) => {
                        tracing::error!("[PHANTOM] ❌ {}", e);
                        return Ok(-2);
                    }
                };

            Ok(spawn_client_with_bound_listener(
                ClientConfig {
                    server_url: url,
                    secret: sec,
                    listen: listen_addr,
                    transport: TransportMode::Auto,
                    ..Default::default()
                },
                listener,
            ) as jint)
        })
        .resolve::<LogErrorAndDefault>()
    }

    #[no_mangle]
    pub extern "system" fn Java_com_resolo_phantom_PhantomTunnel_startMeshClient(
        mut env: EnvUnowned,
        _class: JClass,
        config_json: JString,
        listen_port: jint,
    ) -> jint {
        env.with_env(|env| -> JniResult<jint> {
            let payload = config_json.try_to_string(env)?;
            init_logging();
            match start_mesh_from_json(&payload, listen_port as u16) {
                Ok(port) => Ok(port as jint),
                Err(error) => {
                    tracing::error!("[PHANTOM] ❌ {}", error);
                    Ok(-2)
                }
            }
        })
        .resolve::<LogErrorAndDefault>()
    }

    #[no_mangle]
    pub extern "system" fn Java_com_resolo_phantom_PhantomTunnel_importMeshPeers(
        mut env: EnvUnowned,
        _class: JClass,
        peers_json: JString,
    ) -> jint {
        env.with_env(|env| -> JniResult<jint> {
            let payload = peers_json.try_to_string(env)?;
            let peers: Vec<PacketPeerDescriptor> =
                serde_json::from_str(&payload).map_err(|error| {
                    tracing::error!("[PHANTOM] ❌ mesh peer payload parse failed: {}", error);
                    jni::errors::Error::JniCall(jni::errors::JniError::InvalidArguments)
                })?;
            mesh::import_peers(peers);
            Ok(0)
        })
        .resolve::<LogErrorAndDefault>()
    }

    /// CDN bypass start with full configuration
    #[no_mangle]
    pub extern "system" fn Java_com_resolo_phantom_PhantomTunnel_startClientCdn(
        mut env: EnvUnowned,
        _class: JClass,
        server_url: JString,
        secret: JString,
        listen_port: jint,
        cdn_edge: JString,
        host_override: JString,
        transport_mode: jint,
    ) -> jint {
        env.with_env(|env| -> JniResult<jint> {
            let url = server_url.try_to_string(env)?;
            let sec = secret.try_to_string(env)?;
            let edge_str = cdn_edge.try_to_string(env).unwrap_or_default();
            let host_str = host_override.try_to_string(env).unwrap_or_default();

            let edge = if edge_str.is_empty() {
                None
            } else {
                Some(edge_str)
            };
            let host = if host_str.is_empty() {
                None
            } else {
                Some(host_str)
            };

            let transport = parse_transport_mode_value(transport_mode);
            init_logging();
            let (listener, listen_addr, _actual_port) =
                match bind_requested_or_auto_listener(listen_port as u16) {
                    Ok(bound) => bound,
                    Err(e) => {
                        tracing::error!("[PHANTOM] ❌ {}", e);
                        return Ok(-2);
                    }
                };

            let config = ClientConfig {
                server_url: url,
                secret: sec,
                listen: listen_addr,
                transport: transport.clone(),
                cdn_edge: edge,
                host_override: host,
                fragment: false, // <-- REVERTED: Fragmenting TLS to a WAF often triggers Slowloris protection
                fragment_size: 40,
                padding: true,
                sni_override: None,
                auth_ticket: None,
                mesh_bootstrap: None,
                tls_profile: tls_profile_for_transport(&transport),
                ..Default::default()
            };

            Ok(spawn_client_with_bound_listener(config, listener) as jint)
        })
        .resolve::<LogErrorAndDefault>()
    }

    /// Full configuration start — CDN edge + host override + custom SNI for DPI bypass
    /// Use this for Starlink relay mode or any scenario where SNI spoofing is needed.
    #[no_mangle]
    pub extern "system" fn Java_com_resolo_phantom_PhantomTunnel_startClientFull(
        mut env: EnvUnowned,
        _class: JClass,
        server_url: JString,
        secret: JString,
        listen_port: jint,
        cdn_edge: JString,
        host_override: JString,
        sni_override: JString,
        transport_mode: jint,
        fragment_enabled: jboolean,
        fragment_size: jint,
        obfs_key: JString,
    ) -> jint {
        env.with_env(|env| -> JniResult<jint> {
            let url = server_url.try_to_string(env)?;
            let sec = secret.try_to_string(env)?;
            let edge_str = cdn_edge.try_to_string(env).unwrap_or_default();
            let host_str = host_override.try_to_string(env).unwrap_or_default();
            let sni_str = sni_override.try_to_string(env).unwrap_or_default();
            let obfs_key_str = obfs_key.try_to_string(env).unwrap_or_default();

            let edge = if edge_str.is_empty() {
                None
            } else {
                Some(edge_str)
            };
            let host = if host_str.is_empty() {
                None
            } else {
                Some(host_str)
            };
            let sni = if sni_str.is_empty() {
                None
            } else {
                Some(sni_str)
            };
            let obfs_key = if obfs_key_str.trim().is_empty() {
                None
            } else {
                Some(obfs_key_str)
            };

            let transport = parse_transport_mode_value(transport_mode);
            init_logging();
            let (listener, listen_addr, _actual_port) =
                match bind_requested_or_auto_listener(listen_port as u16) {
                    Ok(bound) => bound,
                    Err(e) => {
                        tracing::error!("[PHANTOM] ❌ {}", e);
                        return Ok(-2);
                    }
                };

            let config = ClientConfig {
                server_url: url,
                secret: sec,
                listen: listen_addr,
                transport: transport.clone(),
                cdn_edge: edge,
                host_override: host,
                sni_override: sni,
                fragment: fragment_enabled,
                fragment_size: fragment_size as usize,
                padding: true,
                auth_ticket: None,
                mesh_bootstrap: None,
                tls_profile: tls_profile_for_transport(&transport),
                obfs_key,
                ..Default::default()
            };

            tracing::info!("[PHANTOM] startClientFull: SNI={:?}", config.sni_override);

            Ok(spawn_client_with_bound_listener(config, listener) as jint)
        })
        .resolve::<LogErrorAndDefault>()
    }

    #[no_mangle]
    pub extern "system" fn Java_com_resolo_phantom_PhantomTunnel_startTun2Socks(
        mut env: EnvUnowned,
        _class: JClass,
        tun_fd: jint,
        socks_address: JString,
        socks_port: jint,
        mtu: jint,
        dns_address: JString,
    ) -> jint {
        env.with_env(|env| -> JniResult<jint> {
            let socks_addr = socks_address.try_to_string(env)?;
            let dns_addr = dns_address.try_to_string(env).unwrap_or_default();
            init_logging();

            let dns = if dns_addr.trim().is_empty() {
                None
            } else {
                Some(dns_addr.as_str())
            };

            match crate::android_tun::start_android_tun_bridge(
                tun_fd,
                &socks_addr,
                socks_port as u16,
                mtu,
                dns,
            ) {
                Ok(()) => Ok(0),
                Err(error) => {
                    tracing::error!("[PHANTOM] ❌ {}", error);
                    Ok(-1)
                }
            }
        })
        .resolve::<LogErrorAndDefault>()
    }

    #[no_mangle]
    pub extern "system" fn Java_com_resolo_phantom_PhantomTunnel_stopTun2Socks(
        _env: EnvUnowned,
        _class: JClass,
    ) {
        init_logging();
        crate::android_tun::stop_android_tun_bridge();
    }
}

fn start_mesh_from_json(payload: &str, listen_port_override: u16) -> Result<i32, String> {
    let request: MeshStartRequest = serde_json::from_str(payload)
        .map_err(|error| format!("Invalid mesh config JSON: {}", error))?;

    if request.ticket.trim().is_empty() {
        return Err("Mesh ticket missing".to_string());
    }

    let requested_port = if listen_port_override != 0 {
        listen_port_override
    } else {
        request.listen_port.unwrap_or(0)
    };

    let (listener, listen_addr, _actual_port) =
        bind_requested_or_auto_listener(requested_port).map_err(|error| error.to_string())?;

    let transport = parse_transport_mode(request.transport_mode.as_deref());
    let tls_profile = request
        .tls_profile
        .as_deref()
        .and_then(parse_tls_profile_name)
        .unwrap_or_else(|| tls_profile_for_transport(&transport));

    let config = ClientConfig {
        server_url: request.server_url,
        secret: String::new(),
        auth_ticket: Some(request.ticket),
        listen: listen_addr,
        transport,
        cdn_edge: request.cdn_edge,
        host_override: request.host_override,
        fragment: request.fragment,
        fragment_size: request.fragment_size.unwrap_or(40),
        padding: true,
        sni_override: request.sni_override,
        mesh_bootstrap: request.bootstrap,
        tls_profile,
        obfs_key: request.obfs_key.filter(|value| !value.trim().is_empty()),
        ..Default::default()
    };

    Ok(spawn_client_with_bound_listener(config, listener))
}

fn parse_transport_mode(value: Option<&str>) -> TransportMode {
    match value.unwrap_or("auto").trim().to_ascii_lowercase().as_str() {
        "ws" | "websocket" => TransportMode::WebSocket,
        "http" => TransportMode::Http,
        "stealth" | "browser" | "browser-like" | "browser_like" => TransportMode::Stealth,
        "obfs" | "ossh" | "raw" => TransportMode::Obfs,
        _ => TransportMode::Auto,
    }
}

fn parse_transport_mode_value(value: i32) -> TransportMode {
    match value {
        1 => TransportMode::WebSocket,
        2 => TransportMode::Http,
        3 => TransportMode::Stealth,
        4 => TransportMode::Obfs,
        _ => TransportMode::Auto,
    }
}

fn tls_profile_for_transport(transport: &TransportMode) -> TlsProfile {
    if matches!(transport, TransportMode::Stealth) {
        TlsProfile::BrowserLike
    } else {
        TlsProfile::Default
    }
}

fn parse_tls_profile_value(value: i32) -> Option<TlsProfile> {
    match value {
        0 => Some(TlsProfile::Default),
        1 => Some(TlsProfile::BrowserLike),
        _ => None,
    }
}

fn parse_tls_profile_name(value: &str) -> Option<TlsProfile> {
    match value.trim().to_ascii_lowercase().as_str() {
        "default" | "rustls" => Some(TlsProfile::Default),
        "browser" | "browser-like" | "browser_like" | "chrome" => Some(TlsProfile::BrowserLike),
        _ => None,
    }
}
