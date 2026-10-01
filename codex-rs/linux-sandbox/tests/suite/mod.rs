// Aggregates all former standalone integration tests as modules.
mod bundled_bwrap;
mod bwrap_path;
mod managed_proxy;
mod sandbox;

fn test_temp_dir() -> tempfile::TempDir {
    let root = std::env::temp_dir()
        .canonicalize()
        .expect("canonical temporary root");
    tempfile::tempdir_in(root).expect("temp dir")
}
