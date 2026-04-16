use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::HashSet;
use std::env;
use std::fs::OpenOptions;
use std::io::Write;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::OnceLock;
use std::sync::RwLock;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

const CODEX_BACKEND_CAPTURE_ENV_VAR: &str = "CODEX_BACKEND_CAPTURE";
const CODEX_BACKEND_CAPTURE_INPUT_ENV_VAR: &str = "CODEX_BACKEND_CAPTURE_INPUT";
const CODEX_BACKEND_CAPTURE_OUTPUT_ENV_VAR: &str = "CODEX_BACKEND_CAPTURE_OUTPUT";
const CODEX_BACKEND_CAPTURE_REASONING_ENV_VAR: &str = "CODEX_BACKEND_CAPTURE_REASONING";
const CODEX_BACKEND_CAPTURE_DIR_ENV_VAR: &str = "CODEX_BACKEND_CAPTURE_DIR";
const CODEX_PROMPT_DEBUG_HTTP_PREFIX: &str = "[codex backend capture]";
const BACKEND_TRAFFIC_FILENAME: &str = "backend_traffic.ndjson";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PromptDebugHttpConfig {
    pub enabled: bool,
    pub capture_input: bool,
    pub capture_output: bool,
    pub capture_reasoning: bool,
    pub capture_dir: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct PromptCaptureSession {
    id: String,
    capture_input: bool,
    capture_output: bool,
    capture_reasoning: bool,
    input_path: PathBuf,
    output_path: PathBuf,
    reasoning_path: PathBuf,
}

impl PromptCaptureSession {
    pub fn id(&self) -> &str {
        self.id.as_str()
    }

    pub fn input_path(&self) -> &Path {
        self.input_path.as_path()
    }

    pub fn output_path(&self) -> &Path {
        self.output_path.as_path()
    }

    pub fn reasoning_path(&self) -> &Path {
        self.reasoning_path.as_path()
    }
}

static PROMPT_DEBUG_HTTP_CONFIG: OnceLock<RwLock<PromptDebugHttpConfig>> = OnceLock::new();
static PROMPT_DEBUG_HTTP_LOG_WRITE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
static PROMPT_DEBUG_HTTP_TOOL_USAGE: OnceLock<Mutex<HashMap<PathBuf, ToolUsageStats>>> =
    OnceLock::new();
static CAPTURE_COUNTER: AtomicU64 = AtomicU64::new(1);
static TRAFFIC_EVENT_COUNTER: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Default)]
struct ToolUsageStats {
    counts: BTreeMap<String, u64>,
    seen_call_ids: HashSet<String>,
}

fn config_lock() -> &'static RwLock<PromptDebugHttpConfig> {
    PROMPT_DEBUG_HTTP_CONFIG.get_or_init(|| RwLock::new(PromptDebugHttpConfig::default()))
}

fn tool_usage_lock() -> &'static Mutex<HashMap<PathBuf, ToolUsageStats>> {
    PROMPT_DEBUG_HTTP_TOOL_USAGE.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn configure_prompt_debug_http(config: PromptDebugHttpConfig) {
    if let Ok(mut guard) = config_lock().write() {
        *guard = config;
    }
}

fn configured_prompt_debug_http() -> PromptDebugHttpConfig {
    config_lock()
        .read()
        .map(|guard| guard.clone())
        .unwrap_or_default()
}

fn env_prompt_debug_http_enabled() -> bool {
    env::var_os(CODEX_BACKEND_CAPTURE_ENV_VAR).is_some()
}

fn env_prompt_debug_http_capture_input() -> Option<bool> {
    env::var_os(CODEX_BACKEND_CAPTURE_INPUT_ENV_VAR).map(|_| true)
}

fn env_prompt_debug_http_capture_output() -> Option<bool> {
    env::var_os(CODEX_BACKEND_CAPTURE_OUTPUT_ENV_VAR).map(|_| true)
}

fn env_prompt_debug_http_capture_reasoning() -> Option<bool> {
    env::var_os(CODEX_BACKEND_CAPTURE_REASONING_ENV_VAR).map(|_| true)
}

fn env_prompt_debug_http_capture_dir() -> Option<PathBuf> {
    env::var_os(CODEX_BACKEND_CAPTURE_DIR_ENV_VAR).map(PathBuf::from)
}

fn active_prompt_debug_http_config() -> PromptDebugHttpConfig {
    let configured = configured_prompt_debug_http();
    let enabled = configured.enabled || env_prompt_debug_http_enabled();
    let capture_input = env_prompt_debug_http_capture_input().unwrap_or(configured.capture_input);
    let capture_output =
        env_prompt_debug_http_capture_output().unwrap_or(configured.capture_output);
    let capture_reasoning =
        env_prompt_debug_http_capture_reasoning().unwrap_or(configured.capture_reasoning);
    let capture_dir = env_prompt_debug_http_capture_dir().or(configured.capture_dir);

    PromptDebugHttpConfig {
        enabled,
        capture_input,
        capture_output,
        capture_reasoning,
        capture_dir,
    }
}

fn default_capture_dir() -> PathBuf {
    PathBuf::from("/tmp")
}

fn next_traffic_event_id() -> u64 {
    TRAFFIC_EVENT_COUNTER.fetch_add(1, Ordering::Relaxed)
}

fn next_capture_id() -> String {
    let seq = CAPTURE_COUNTER.fetch_add(1, Ordering::Relaxed);
    seq.to_string()
}

fn append_line(path: &Path, line: &str) {
    let write_lock = PROMPT_DEBUG_HTTP_LOG_WRITE_LOCK.get_or_init(|| Mutex::new(()));
    let Ok(_guard) = write_lock.lock() else {
        return;
    };

    if let Some(parent) = path.parent()
        && std::fs::create_dir_all(parent).is_err()
    {
        return;
    }

    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{line}");
    }
}

fn append_json_line(path: &Path, value: &serde_json::Value) {
    append_line(path, &value.to_string());
}

fn capture_traffic_path(dir: &Path) -> PathBuf {
    dir.join(BACKEND_TRAFFIC_FILENAME)
}

fn now_unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0)
}

pub fn capture_headers_json(headers: &http::HeaderMap) -> serde_json::Value {
    let mut entries = serde_json::Map::new();
    for name in headers.keys() {
        let values: Vec<String> = headers
            .get_all(name)
            .iter()
            .map(|value| String::from_utf8_lossy(value.as_bytes()).to_string())
            .collect();
        entries.insert(name.to_string(), serde_json::json!(values));
    }
    serde_json::Value::Object(entries)
}

pub fn backend_capture_append_event(mut event: serde_json::Value) {
    let config = active_prompt_debug_http_config();
    if !config.enabled {
        return;
    }

    let mut map = match event {
        serde_json::Value::Object(map) => map,
        other => {
            let mut map = serde_json::Map::new();
            map.insert("payload".to_string(), other);
            map
        }
    };
    map.insert(
        "event_seq".to_string(),
        serde_json::json!(next_traffic_event_id()),
    );
    map.insert(
        "timestamp_unix_ms".to_string(),
        serde_json::json!(now_unix_ms()),
    );
    event = serde_json::Value::Object(map);

    let dir = config.capture_dir.unwrap_or_else(default_capture_dir);
    if std::fs::create_dir_all(dir.as_path()).is_err() {
        return;
    }

    let path = capture_traffic_path(dir.as_path());
    append_json_line(path.as_path(), &event);
}

fn tool_name_from_call(map: &serde_json::Map<String, serde_json::Value>) -> Option<String> {
    let item_type = map.get("type").and_then(serde_json::Value::as_str)?;
    if !item_type.ends_with("_call") {
        return None;
    }

    if let Some(name) = map.get("name").and_then(serde_json::Value::as_str) {
        return Some(name.to_string());
    }

    if let Some(name) = map
        .get("function")
        .and_then(serde_json::Value::as_object)
        .and_then(|function| function.get("name"))
        .and_then(serde_json::Value::as_str)
    {
        return Some(name.to_string());
    }

    Some(item_type.trim_end_matches("_call").to_string())
}

fn collect_tool_calls(value: &serde_json::Value, out: &mut Vec<(String, Option<String>)>) {
    match value {
        serde_json::Value::Array(items) => {
            for item in items {
                collect_tool_calls(item, out);
            }
        }
        serde_json::Value::Object(map) => {
            if let Some(name) = tool_name_from_call(map) {
                let call_id = map
                    .get("call_id")
                    .or_else(|| map.get("id"))
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned);
                out.push((name, call_id));
            }

            for child in map.values() {
                collect_tool_calls(child, out);
            }
        }
        _ => {}
    }
}

fn stats_path_for_session(session: &PromptCaptureSession) -> Option<PathBuf> {
    session
        .output_path()
        .parent()
        .map(|parent| parent.join("tool_usage_stats.json"))
}

fn write_tool_usage_stats_file(stats_path: &Path, stats: &ToolUsageStats) {
    let updated_unix_seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);

    let payload = serde_json::json!({
        "updated_unix_seconds": updated_unix_seconds,
        "total_calls": stats.counts.values().copied().sum::<u64>(),
        "distinct_tools": stats.counts.len(),
        "tool_counts": stats.counts,
        "dedupe": "call_id per process runtime",
    });

    let Ok(serialized) = serde_json::to_string_pretty(&payload) else {
        return;
    };

    if let Ok(mut file) = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(stats_path)
    {
        let _ = writeln!(file, "{serialized}");
    }
}

fn record_tool_usage(session: &PromptCaptureSession, payload: &serde_json::Value) {
    let Some(stats_path) = stats_path_for_session(session) else {
        return;
    };

    let mut calls = Vec::new();
    collect_tool_calls(payload, &mut calls);
    if calls.is_empty() {
        return;
    }

    let write_lock = PROMPT_DEBUG_HTTP_LOG_WRITE_LOCK.get_or_init(|| Mutex::new(()));
    let Ok(_write_guard) = write_lock.lock() else {
        return;
    };

    let Ok(mut usage_guard) = tool_usage_lock().lock() else {
        return;
    };

    let stats = usage_guard
        .entry(stats_path.clone())
        .or_insert_with(ToolUsageStats::default);
    let mut changed = false;

    for (tool_name, call_id) in calls {
        if let Some(call_id) = call_id
            && !stats.seen_call_ids.insert(call_id)
        {
            continue;
        }
        *stats.counts.entry(tool_name).or_insert(0) += 1;
        changed = true;
    }

    if changed {
        write_tool_usage_stats_file(stats_path.as_path(), stats);
    }
}

pub fn prompt_debug_http_enabled() -> bool {
    active_prompt_debug_http_config().enabled
}

pub fn start_prompt_capture(kind: &str, input: Option<&str>) -> Option<PromptCaptureSession> {
    let config = active_prompt_debug_http_config();
    if !config.enabled {
        return None;
    }

    let id = next_capture_id();
    let dir = config.capture_dir.unwrap_or_else(default_capture_dir);
    if std::fs::create_dir_all(dir.as_path()).is_err() {
        return None;
    }

    let input_path = dir.join(format!("{id}_input.ndjson"));
    let output_path = dir.join(format!("{id}_output.ndjson"));
    let reasoning_path = dir.join(format!("{id}_reasoning.ndjson"));

    // Ensure a stats file exists for this capture directory even before first tool call.
    let stats_path = dir.join("tool_usage_stats.json");
    {
        let write_lock = PROMPT_DEBUG_HTTP_LOG_WRITE_LOCK.get_or_init(|| Mutex::new(()));
        if let Ok(_write_guard) = write_lock.lock()
            && let Ok(mut usage_guard) = tool_usage_lock().lock()
        {
            let stats = usage_guard
                .entry(stats_path.clone())
                .or_insert_with(ToolUsageStats::default);
            write_tool_usage_stats_file(stats_path.as_path(), stats);
        }
    }

    if config.capture_input
        && let Some(payload) = input
    {
        append_json_line(
            input_path.as_path(),
            &serde_json::json!({
                "kind": kind,
                "query_id": id,
                "transport": kind,
                "payload": payload,
            }),
        );
    }

    Some(PromptCaptureSession {
        id,
        capture_input: config.capture_input,
        capture_output: config.capture_output,
        capture_reasoning: config.capture_reasoning,
        input_path,
        output_path,
        reasoning_path,
    })
}

pub fn prompt_capture_append_input(session: &PromptCaptureSession, transport: &str, payload: &str) {
    if !session.capture_input {
        return;
    }
    append_json_line(
        session.input_path(),
        &serde_json::json!({
            "query_id": session.id(),
            "transport": transport,
            "payload": payload,
        }),
    );
}

pub fn prompt_capture_append_output(
    session: &PromptCaptureSession,
    transport: &str,
    payload: &str,
) {
    if !session.capture_output {
        return;
    }
    append_json_line(
        session.output_path(),
        &serde_json::json!({
            "query_id": session.id(),
            "transport": transport,
            "payload": payload,
        }),
    );

    if let Ok(json_payload) = serde_json::from_str::<serde_json::Value>(payload) {
        record_tool_usage(session, &json_payload);
    }
}

pub fn prompt_capture_append_reasoning(
    session: &PromptCaptureSession,
    transport: &str,
    payload: &str,
) {
    if !session.capture_reasoning {
        return;
    }
    append_json_line(
        session.reasoning_path(),
        &serde_json::json!({
            "query_id": session.id(),
            "transport": transport,
            "payload": payload,
        }),
    );
}

pub fn prompt_capture_write_output_json(
    session: Option<&PromptCaptureSession>,
    label: &str,
    json: &serde_json::Value,
) {
    let Some(session) = session else {
        return;
    };
    if !session.capture_output {
        return;
    }

    append_json_line(
        session.output_path(),
        &serde_json::json!({
            "query_id": session.id(),
            "transport": "structured_output",
            "label": label,
            "payload": json,
        }),
    );

    record_tool_usage(session, json);
}

pub fn capture_output_enabled() -> bool {
    let config = active_prompt_debug_http_config();
    config.enabled && config.capture_output
}

pub fn capture_dir() -> Option<PathBuf> {
    let config = active_prompt_debug_http_config();
    if !config.enabled {
        return None;
    }
    Some(config.capture_dir.unwrap_or_else(default_capture_dir))
}

pub fn prompt_debug_http_log(message: impl AsRef<str>) {
    if !prompt_debug_http_enabled() {
        return;
    }

    eprintln!("{CODEX_PROMPT_DEBUG_HTTP_PREFIX} {}", message.as_ref());
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn collect_tool_calls_includes_function_and_custom_calls() {
        let payload = serde_json::json!({
            "output": [
                {
                    "type": "function_call",
                    "name": "exec_command",
                    "call_id": "call_1"
                },
                {
                    "type": "custom_tool_call",
                    "name": "apply_patch",
                    "call_id": "call_2"
                }
            ]
        });

        let mut calls = Vec::new();
        collect_tool_calls(&payload, &mut calls);
        assert_eq!(
            calls,
            vec![
                ("exec_command".to_string(), Some("call_1".to_string())),
                ("apply_patch".to_string(), Some("call_2".to_string())),
            ]
        );
    }

    #[test]
    fn collect_tool_calls_falls_back_to_type_stem_when_name_missing() {
        let payload = serde_json::json!({
            "type": "web_search_call",
            "id": "ws_123"
        });

        let mut calls = Vec::new();
        collect_tool_calls(&payload, &mut calls);
        assert_eq!(calls, vec![("web_search".to_string(), Some("ws_123".to_string()))]);
    }
}
