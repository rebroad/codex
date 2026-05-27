use std::env;
use std::fs;
use std::path::PathBuf;

fn read_version() -> String {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let version_path = manifest_dir.join("../VERSION");

    if let Ok(version) = fs::read_to_string(&version_path) {
        let version = version.trim();
        if !version.is_empty() {
            return version.to_string();
        }
    }

    env::var("CARGO_PKG_VERSION").expect("CARGO_PKG_VERSION")
}

fn main() {
    println!("cargo:rerun-if-changed=../VERSION");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rustc-env=CODEX_PROXY_VERSION={}", read_version());
}
