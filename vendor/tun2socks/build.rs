use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("Missing manifest dir"));
    let source_impl_dir = manifest_dir.join("impl");
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("Missing OUT_DIR"));
    let build_impl_dir = out_dir.join("tun2socks-impl");
    let target = env::var("TARGET").expect("Missing TARGET");
    let target_env = target.replace('-', "_");

    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=impl");
    println!("cargo:rerun-if-env-changed=TARGET");
    println!("cargo:rerun-if-env-changed=NUM_JOBS");
    println!("cargo:rerun-if-env-changed=CC_{target_env}");
    println!("cargo:rerun-if-env-changed=AR_{target_env}");
    println!("cargo:rerun-if-env-changed=CXX_{target_env}");

    if build_impl_dir.exists() {
        fs::remove_dir_all(&build_impl_dir).expect("Failed to reset tun2socks build dir");
    }
    copy_impl_tree(&source_impl_dir, &build_impl_dir).expect("Failed to stage tun2socks sources");

    let jobs = env::var("NUM_JOBS").unwrap_or_else(|_| "8".to_string());
    let mut make = Command::new("make");
    make.arg("static")
        .arg(format!("-j{jobs}"))
        .current_dir(&build_impl_dir);

    if target.contains("android") {
        let cc = env::var(format!("CC_{target_env}"))
            .unwrap_or_else(|_| panic!("Missing CC_{target_env} for Android tun2socks build"));
        let ar = env::var(format!("AR_{target_env}"))
            .unwrap_or_else(|_| panic!("Missing AR_{target_env} for Android tun2socks build"));

        make.arg(format!("CC={cc}"))
            .arg(format!("PP={cc}"))
            .arg(format!("LD={cc}"))
            .arg(format!("AR={ar}"));
    }

    let status = make.status().expect("Failed to build tun2socks");
    assert!(status.success(), "tun2socks make static failed with {status}");

    println!(
        "cargo:rustc-link-search=native={}",
        build_impl_dir.join("bin").display()
    );
    println!("cargo:rustc-link-lib=static=hev-socks5-tunnel");
    println!(
        "cargo:rustc-link-search=native={}",
        build_impl_dir.join("third-part/hev-task-system/bin").display()
    );
    println!("cargo:rustc-link-lib=static=hev-task-system");
    println!(
        "cargo:rustc-link-search=native={}",
        build_impl_dir.join("third-part/lwip/bin").display()
    );
    println!("cargo:rustc-link-lib=static=lwip");
    println!(
        "cargo:rustc-link-search=native={}",
        build_impl_dir.join("third-part/yaml/bin").display()
    );
    println!("cargo:rustc-link-lib=static=yaml");
}

fn copy_impl_tree(source: &Path, target: &Path) -> io::Result<()> {
    fs::create_dir_all(target)?;

    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let entry_path = entry.path();
        let target_path = target.join(entry.file_name());
        let file_type = entry.file_type()?;

        if file_type.is_dir() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name == "bin" || name == "build" {
                continue;
            }

            copy_impl_tree(&entry_path, &target_path)?;
        } else if file_type.is_file() {
            fs::copy(&entry_path, &target_path)?;
        }
    }

    Ok(())
}
