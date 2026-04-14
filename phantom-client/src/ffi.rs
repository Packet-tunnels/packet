use crate::{start_client, start_client_with_config, stats, ClientConfig, TransportMode};
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

#[no_mangle]
pub extern "C" fn phantom_emit_test_output() {
    init_logging();
    tracing::info!("[PHANTOM] iOS test bridge is active");
    tracing::info!("[PHANTOM] Rust log callback delivered output to SwiftUI");
}

#[no_mangle]
pub extern "C" fn phantom_copy_stats_json() -> *mut c_char {
    let json = stats::snapshot_json();
    CString::new(json)
        .unwrap_or_else(|_| CString::new("{}").unwrap())
        .into_raw()
}

#[no_mangle]
pub extern "C" fn phantom_free_string(value: *mut c_char) {
    if value.is_null() {
        return;
    }

    unsafe {
        let _ = CString::from_raw(value);
    }
}

// ─── C API (iOS & General FFI) ─────────────────────────────────

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
    use jni::errors::LogErrorAndDefault;
    use jni::objects::{JClass, JObject};
    use jni::sys::jint;
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

    /// Basic start (backward compatible)
    #[no_mangle]
    pub extern "system" fn Java_com_resolo_phantom_PhantomTunnel_startClient(
        mut env: EnvUnowned,
        _class: JClass,
        server_url: JString,
        secret: JString,
        listen_port: jint,
    ) {
        env.with_env(|env| -> JniResult<()> {
            let url = server_url.try_to_string(env)?;
            let sec = secret.try_to_string(env)?;
            let listen_addr = format!("127.0.0.1:{}", listen_port);
            init_logging();

            thread::spawn(move || {
                let rt = Runtime::new().expect("Failed to create tokio runtime");
                rt.block_on(async {
                    start_client(url, sec, listen_addr).await;
                });
            });

            Ok(())
        })
        .resolve::<LogErrorAndDefault>();
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
    ) {
        env.with_env(|env| -> JniResult<()> {
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

            let transport = match transport_mode {
                1 => TransportMode::WebSocket,
                2 => TransportMode::Http,
                _ => TransportMode::Auto,
            };
            init_logging();

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

            Ok(())
        })
        .resolve::<LogErrorAndDefault>();
    }
}
