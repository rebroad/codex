use std::env;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_env = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    let uses_gstreamer = (target_os == "linux" && target_env == "gnu")
        || (target_os == "windows" && target_env == "msvc")
        || target_os == "macos";

    if uses_gstreamer {
        system_deps::Config::new().probe()?;
    }

    Ok(())
}
