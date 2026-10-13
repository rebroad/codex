use std::fs;

use serde_json::json;

use super::{
    move_cache_diagnostics_for_rollout, remove_cache_diagnostics_for_rollout,
    write_cache_diagnostics,
};

#[test]
fn diagnostics_are_written_and_removed_beside_rollout() {
    let directory = tempfile::tempdir().expect("create temp directory");
    let rollout = directory
        .path()
        .join("rollout-2026-10-13T13-48-55-01a1354f-0dc5-7463-940b-c320980efc69.jsonl");
    let sidecar = rollout.with_file_name(format!(
        "{}.cache-diagnostics.json",
        rollout.file_name().unwrap().to_string_lossy()
    ));

    write_cache_diagnostics(
        &rollout,
        &json!({ "cache_key": "key", "fingerprint": "abc" }),
    )
    .expect("write diagnostics");
    assert_eq!(
        fs::read_to_string(&sidecar).expect("read sidecar"),
        r#"{"cache_key":"key","fingerprint":"abc"}"#
    );

    remove_cache_diagnostics_for_rollout(&rollout).expect("remove sidecar");
    assert!(!sidecar.exists());
    remove_cache_diagnostics_for_rollout(&rollout).expect("missing sidecar is fine");
}

#[test]
fn diagnostics_move_with_rollout() {
    let directory = tempfile::tempdir().expect("create temp directory");
    let source_dir = directory.path().join("source");
    let destination_dir = directory.path().join("destination");
    fs::create_dir_all(&source_dir).expect("create source directory");
    fs::create_dir_all(&destination_dir).expect("create destination directory");
    let rollout_name = "rollout-2026-10-13T13-48-55-01a1354f-0dc5-7463-940b-c320980efc69.jsonl";
    let source = source_dir.join(rollout_name);
    let destination = destination_dir.join(rollout_name);
    write_cache_diagnostics(&source, &json!({ "cache_key": "key" })).expect("write diagnostics");
    fs::write(&source, "rollout").expect("create rollout");
    fs::rename(&source, &destination).expect("move rollout");

    move_cache_diagnostics_for_rollout(&source, &destination).expect("move diagnostics");

    let source_sidecar = source.with_file_name(format!("{rollout_name}.cache-diagnostics.json"));
    let destination_sidecar =
        destination.with_file_name(format!("{rollout_name}.cache-diagnostics.json"));
    assert!(!source_sidecar.exists());
    assert_eq!(
        fs::read_to_string(destination_sidecar).expect("read moved diagnostics"),
        r#"{"cache_key":"key"}"#
    );
}
