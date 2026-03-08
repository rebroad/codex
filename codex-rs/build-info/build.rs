use chrono::Local;

fn main() {
    let base_version = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "unknown".to_string());
    let timestamp = build_timestamp_yyyymmddhhmm();
    println!("cargo:rustc-env=CODEX_BUILD_VERSION={base_version}-{timestamp}");
}

fn build_timestamp_yyyymmddhhmm() -> String {
    // Use local time so timestamps reflect the user's timezone.
    Local::now().format("%Y%m%d%H%M").to_string()
}
