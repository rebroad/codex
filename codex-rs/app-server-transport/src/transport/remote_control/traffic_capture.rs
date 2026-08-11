use super::RemoteControlTrafficLogRedaction;
use codex_utils_absolute_path::AbsolutePathBuf;
use serde_json::Value;
use std::fs::File;
use std::fs::OpenOptions;
use std::io::BufWriter;
use std::io::Write;
use std::sync::Mutex;
use std::time::Duration;
use std::time::Instant;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;
use tracing::warn;

const REDACTED: &str = "[REDACTED]";
const CAPTURE_BUFFER_CAPACITY: usize = 256 * 1024;
const CAPTURE_FLUSH_INTERVAL: Duration = Duration::from_millis(500);

struct CaptureWriter {
    writer: BufWriter<File>,
    last_flush: Instant,
}

pub(crate) struct RemoteControlTrafficCapture {
    writer: Mutex<CaptureWriter>,
    redaction: RemoteControlTrafficLogRedaction,
}

impl RemoteControlTrafficCapture {
    pub(crate) fn open(
        path_prefix: Option<&AbsolutePathBuf>,
        redaction: RemoteControlTrafficLogRedaction,
    ) -> std::io::Result<Option<Self>> {
        let Some(path_prefix) = path_prefix else {
            return Ok(None);
        };
        let path_prefix = path_prefix.to_path_buf();
        if let Some(parent) = path_prefix.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let timestamp_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let file_name = path_prefix
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("remote-control")
            .to_string();
        let path = path_prefix.with_file_name(format!(
            "{file_name}.{timestamp_ms}.{}.jsonl",
            std::process::id()
        ));
        let file = OpenOptions::new().create_new(true).write(true).open(path)?;
        Ok(Some(Self {
            writer: Mutex::new(CaptureWriter {
                writer: BufWriter::with_capacity(CAPTURE_BUFFER_CAPACITY, file),
                last_flush: Instant::now(),
            }),
            redaction,
        }))
    }

    pub(crate) fn captures_raw_wire(&self) -> bool {
        self.redaction == RemoteControlTrafficLogRedaction::Disabled
    }

    pub(crate) fn record(&self, direction: &str, frame: Value) {
        let mut record = serde_json::json!({
            "timestampMs": SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis(),
            "direction": direction,
            "frame": frame,
        });
        redact_value(
            record
                .get_mut("frame")
                .expect("capture record always contains a frame"),
            self.redaction,
        );
        let mut writer = match self.writer.lock() {
            Ok(writer) => writer,
            Err(_) => {
                warn!("remote-control traffic capture lock is poisoned");
                return;
            }
        };
        if serde_json::to_writer(&mut writer.writer, &record).is_err()
            || writer.writer.write_all(b"\n").is_err()
        {
            warn!("failed to write remote-control traffic capture");
        }
        flush_if_due(&mut writer);
    }

    pub(crate) fn record_raw(&self, direction: &str, frame: &str) {
        debug_assert!(self.captures_raw_wire());
        let mut writer = match self.writer.lock() {
            Ok(writer) => writer,
            Err(_) => {
                warn!("remote-control traffic capture lock is poisoned");
                return;
            }
        };
        let timestamp_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let direction = match serde_json::to_string(direction) {
            Ok(direction) => direction,
            Err(_) => {
                warn!("failed to encode remote-control traffic direction");
                return;
            }
        };
        let write_result = (|| {
            writer.writer.write_all(b"{\"timestampMs\":")?;
            writer
                .writer
                .write_all(timestamp_ms.to_string().as_bytes())?;
            writer.writer.write_all(b",\"direction\":")?;
            writer.writer.write_all(direction.as_bytes())?;
            writer.writer.write_all(b",\"frame\":")?;
            writer.writer.write_all(frame.as_bytes())?;
            writer.writer.write_all(b"}\n")
        })();
        if write_result.is_err() {
            warn!("failed to write remote-control traffic capture");
        }
        flush_if_due(&mut writer);
    }

    pub(crate) fn flush(&self) {
        let Ok(mut writer) = self.writer.lock() else {
            warn!("remote-control traffic capture lock is poisoned");
            return;
        };
        if writer.writer.flush().is_err() {
            warn!("failed to flush remote-control traffic capture");
        }
    }
}

impl Drop for RemoteControlTrafficCapture {
    fn drop(&mut self) {
        if let Ok(writer) = self.writer.get_mut() {
            let _ = writer.writer.flush();
        }
    }
}

fn flush_if_due(writer: &mut CaptureWriter) {
    if writer.last_flush.elapsed() >= CAPTURE_FLUSH_INTERVAL {
        if writer.writer.flush().is_err() {
            warn!("failed to flush remote-control traffic capture");
        }
        writer.last_flush = Instant::now();
    }
}

fn redact_value(value: &mut Value, redaction: RemoteControlTrafficLogRedaction) {
    match value {
        Value::Object(fields) => {
            for (key, value) in fields {
                if is_secret_key(key)
                    || (redaction == RemoteControlTrafficLogRedaction::Content
                        && is_content_key(key))
                {
                    *value = Value::String(REDACTED.to_string());
                } else {
                    redact_value(value, redaction);
                }
            }
        }
        Value::Array(values) => {
            for value in values {
                redact_value(value, redaction);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

fn is_secret_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    key.contains("authorization")
        || key.contains("access_token")
        || key.contains("accesstoken")
        || key.contains("refresh_token")
        || key.contains("refreshtoken")
        || key.contains("remote_control_token")
        || key.contains("remotecontroltoken")
        || key == "token"
        || key.contains("cookie")
        || key.contains("password")
        || key.contains("secret")
        || key == "bearer"
}

fn is_content_key(key: &str) -> bool {
    matches!(
        key.to_ascii_lowercase().as_str(),
        "arguments" | "command" | "content" | "input" | "output" | "prompt" | "text"
    )
}

#[cfg(test)]
#[path = "traffic_capture_tests.rs"]
mod tests;
