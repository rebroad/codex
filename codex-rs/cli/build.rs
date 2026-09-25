fn main() {
    println!("cargo:rerun-if-env-changed=CODEX_BUILD_TIMESTAMP");
    let suffix = std::env::var("CODEX_BUILD_TIMESTAMP")
        .unwrap_or_else(|_| "000000000000-000000000000".to_string());
    println!(
        "cargo:rustc-env=CODEX_BUILD_VERSION={}-{}",
        env!("CARGO_PKG_VERSION"),
        suffix
    );
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rustc-link-arg=-ObjC");
    }
}
