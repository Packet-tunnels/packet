use crate::{bind_socks_listener, start_client_with_listener, ClientConfig, TransportMode};
use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::sync::Mutex;
use std::thread;
use tokio::runtime::Runtime;
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

    thread::spawn(move || {
        let rt = Runtime::new().expect("Failed to create tokio runtime");
        rt.block_on(async {
            start_client_with_listener(config, listener).await;
        });
    });

    i32::from(actual_port)
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

    let transport = match transport_mode {
        1 => TransportMode::WebSocket,
        2 => TransportMode::Http,
        _ => TransportMode::Auto,
    };

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
        transport,
        cdn_edge: edge,
        host_override: host,
        fragment: true,     // <-- ENABLED FOR IR DPI BYPASS
        fragment_size: 40,
        padding: true,
        sni_override: None,
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
    use jni::sys::{jint, jstring};
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

            let edge = if edge_str.is_empty() { None } else { Some(edge_str) };
            let host = if host_str.is_empty() { None } else { Some(host_str) };

            let transport = match transport_mode {
                1 => TransportMode::WebSocket,
                2 => TransportMode::Http,
                _ => TransportMode::Auto,
            };
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
                transport,
                cdn_edge: edge,
                host_override: host,
                fragment: true,     // <-- ENABLED FOR IR DPI BYPASS
                fragment_size: 40,
                padding: true,
                sni_override: None,
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
    ) -> jint {
        env.with_env(|env| -> JniResult<jint> {
            let url = server_url.try_to_string(env)?;
            let sec = secret.try_to_string(env)?;
            let edge_str = cdn_edge.try_to_string(env).unwrap_or_default();
            let host_str = host_override.try_to_string(env).unwrap_or_default();
            let sni_str = sni_override.try_to_string(env).unwrap_or_default();

            let edge = if edge_str.is_empty() { None } else { Some(edge_str) };
            let host = if host_str.is_empty() { None } else { Some(host_str) };
            let sni = if sni_str.is_empty() { None } else { Some(sni_str) };

            let transport = match transport_mode {
                1 => TransportMode::WebSocket,
                2 => TransportMode::Http,
                _ => TransportMode::Auto,
            };
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
                transport,
                cdn_edge: edge,
                host_override: host,
                sni_override: sni,
                fragment: true,     // <-- ENABLED FOR IR DPI BYPASS
                fragment_size: 40,
                padding: true,
            };

            tracing::info!(
                "[PHANTOM] startClientFull: SNI={:?}",
                config.sni_override
            );

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
