use std::ffi::CStr;
use std::os::raw::c_char;
use std::thread;
use tokio::runtime::Runtime;
use crate::start_client;

// ─── C API (iOS & General FFI) ─────────────────────────────────

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

// ─── JNI (Android) ─────────────────────────────────────────────
#[cfg(target_os = "android")]
#[allow(non_snake_case)]
pub mod android {
    use super::*;
    use jni::JNIEnv;
    use jni::objects::{JClass, JString};
    use jni::sys::jint;

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
}
