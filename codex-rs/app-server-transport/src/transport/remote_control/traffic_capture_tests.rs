use super::*;
use codex_utils_absolute_path::test_support::PathExt;
use serde_json::json;
use tempfile::TempDir;

#[test]
fn records_redacted_jsonl() {
    let temp_dir = TempDir::new().expect("temporary directory should be created");
    let capture = RemoteControlTrafficCapture::open(
        Some(&temp_dir.path().join("traffic").abs()),
        RemoteControlTrafficLogRedaction::Secrets,
    )
    .expect("capture should open")
    .expect("capture should be enabled");

    capture.record(
        "inbound",
        json!({
            "method": "thread/start",
            "params": {
                "accessToken": "secret-token",
                "prompt": "keep this content",
            },
        }),
    );
    capture.flush();

    let path = std::fs::read_dir(temp_dir.path())
        .expect("capture directory should be readable")
        .next()
        .expect("capture file should exist")
        .expect("capture entry should be readable")
        .path();
    let line = std::fs::read_to_string(path).expect("capture file should be readable");
    let record: serde_json::Value = serde_json::from_str(line.trim()).expect("valid JSONL");
    assert_eq!(record["direction"], "inbound");
    assert_eq!(record["frame"]["params"]["accessToken"], "[REDACTED]");
    assert_eq!(record["frame"]["params"]["prompt"], "keep this content");
    assert!(record["timestampMs"].is_number());
}

#[test]
fn content_redaction_hides_payload_fields() {
    let temp_dir = TempDir::new().expect("temporary directory should be created");
    let capture = RemoteControlTrafficCapture::open(
        Some(&temp_dir.path().join("traffic").abs()),
        RemoteControlTrafficLogRedaction::Content,
    )
    .expect("capture should open")
    .expect("capture should be enabled");

    capture.record(
        "outbound",
        json!({"text": "sensitive", "nested": {"token": "secret"}}),
    );
    capture.flush();

    let path = std::fs::read_dir(temp_dir.path())
        .expect("capture directory should be readable")
        .next()
        .expect("capture file should exist")
        .expect("capture entry should be readable")
        .path();
    let line = std::fs::read_to_string(path).expect("capture file should be readable");
    let record: serde_json::Value = serde_json::from_str(line.trim()).expect("valid JSONL");
    assert_eq!(record["frame"]["text"], "[REDACTED]");
    assert_eq!(record["frame"]["nested"]["token"], "[REDACTED]");
}

#[test]
fn raw_capture_preserves_wire_json() {
    let temp_dir = TempDir::new().expect("temporary directory should be created");
    let capture = RemoteControlTrafficCapture::open(
        Some(&temp_dir.path().join("traffic").abs()),
        RemoteControlTrafficLogRedaction::Disabled,
    )
    .expect("capture should open")
    .expect("capture should be enabled");
    let wire_json = r#"{"jsonrpc":"2.0", "id":7, "params":{"text":"keep"}}"#;

    capture.record_raw("inbound", wire_json);
    capture.flush();

    let path = std::fs::read_dir(temp_dir.path())
        .expect("capture directory should be readable")
        .next()
        .expect("capture file should exist")
        .expect("capture entry should be readable")
        .path();
    let line = std::fs::read_to_string(path).expect("capture file should be readable");
    let expected_suffix = format!(",\"direction\":\"inbound\",\"frame\":{wire_json}}}\n");
    assert!(line.ends_with(&expected_suffix), "capture line: {line:?}");
}
