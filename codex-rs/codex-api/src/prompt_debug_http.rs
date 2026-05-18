use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::HashSet;
use std::env;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::OnceLock;
use std::sync::RwLock;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use codex_protocol::models::ResponseItem;

const CODEX_BACKEND_CAPTURE_ENV_VAR: &str = "CODEX_BACKEND_CAPTURE";
const CODEX_BACKEND_CAPTURE_INPUT_ENV_VAR: &str = "CODEX_BACKEND_CAPTURE_INPUT";
const CODEX_BACKEND_CAPTURE_OUTPUT_ENV_VAR: &str = "CODEX_BACKEND_CAPTURE_OUTPUT";
const CODEX_BACKEND_CAPTURE_DIR_ENV_VAR: &str = "CODEX_BACKEND_CAPTURE_DIR";
const CODEX_BACKEND_CAPTURE_STDERR_ENV_VAR: &str = "CODEX_BACKEND_CAPTURE_STDERR";
const CODEX_PROMPT_DEBUG_HTTP_PREFIX: &str = "[codex backend capture]";
const EMAIL_PLACEHOLDER: &str = "$EMAIL";
const QUERY_ID_COUNTER_FILENAME: &str = ".query_id_counter";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PromptDebugHttpConfig {
    pub enabled: bool,
    pub capture_input: bool,
    pub capture_output: bool,
    pub capture_dir: Option<PathBuf>,
    pub tool_usage_log: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct PromptCaptureSession {
    id: String,
    capture_input: bool,
    capture_output: bool,
    backend_traffic_path: PathBuf,
    input_path: PathBuf,
    output_path: PathBuf,
    tool_usage_log_path: PathBuf,
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

    pub fn backend_traffic_path(&self) -> &Path {
        self.backend_traffic_path.as_path()
    }

    pub fn tool_usage_log_path(&self) -> &Path {
        self.tool_usage_log_path.as_path()
    }
}

static PROMPT_DEBUG_HTTP_CONFIG: OnceLock<RwLock<PromptDebugHttpConfig>> = OnceLock::new();
static PROMPT_DEBUG_HTTP_ACCOUNT_EMAIL: OnceLock<RwLock<Option<String>>> = OnceLock::new();
static PROMPT_DEBUG_HTTP_LOG_WRITE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
static PROMPT_DEBUG_HTTP_TOOL_USAGE: OnceLock<Mutex<HashMap<PathBuf, ToolUsageStats>>> =
    OnceLock::new();
static TRAFFIC_EVENT_COUNTER: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Default)]
struct ToolUsageStats {
    counts: BTreeMap<String, u64>,
    failure_counts: BTreeMap<String, u64>,
    seen_call_ids: HashSet<String>,
    seen_failure_call_ids: HashSet<String>,
    tool_names_by_call_id: HashMap<String, String>,
}

fn config_lock() -> &'static RwLock<PromptDebugHttpConfig> {
    PROMPT_DEBUG_HTTP_CONFIG.get_or_init(|| RwLock::new(PromptDebugHttpConfig::default()))
}

fn tool_usage_lock() -> &'static Mutex<HashMap<PathBuf, ToolUsageStats>> {
    PROMPT_DEBUG_HTTP_TOOL_USAGE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn account_email_lock() -> &'static RwLock<Option<String>> {
    PROMPT_DEBUG_HTTP_ACCOUNT_EMAIL.get_or_init(|| RwLock::new(None))
}

pub fn configure_prompt_debug_http(config: PromptDebugHttpConfig) {
    if let Ok(mut guard) = config_lock().write() {
        *guard = config;
    }
}

pub fn set_prompt_debug_http_account_email(account_email: Option<String>) {
    if let Ok(mut guard) = account_email_lock().write() {
        *guard = account_email;
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

fn env_prompt_debug_http_capture_dir() -> Option<PathBuf> {
    env::var_os(CODEX_BACKEND_CAPTURE_DIR_ENV_VAR).map(PathBuf::from)
}

fn active_prompt_debug_http_config() -> PromptDebugHttpConfig {
    let configured = configured_prompt_debug_http();
    let enabled = configured.enabled || env_prompt_debug_http_enabled();
    let capture_input = env_prompt_debug_http_capture_input().unwrap_or(configured.capture_input);
    let capture_output =
        env_prompt_debug_http_capture_output().unwrap_or(configured.capture_output);
    let capture_dir = env_prompt_debug_http_capture_dir().or(configured.capture_dir);

    PromptDebugHttpConfig {
        enabled,
        capture_input,
        capture_output,
        capture_dir,
        tool_usage_log: configured.tool_usage_log,
    }
}

fn default_capture_dir() -> PathBuf {
    PathBuf::from("/tmp")
}

fn next_traffic_event_id() -> u64 {
    TRAFFIC_EVENT_COUNTER.fetch_add(1, Ordering::Relaxed)
}

fn configured_account_email() -> Option<String> {
    account_email_lock()
        .read()
        .ok()
        .and_then(|guard| guard.clone())
}

fn resolve_prompt_debug_path(path: PathBuf) -> PathBuf {
    let path_str = path.to_string_lossy();
    if !path_str.contains(EMAIL_PLACEHOLDER) {
        return path;
    }
    let Some(account_email) = configured_account_email() else {
        return path;
    };
    PathBuf::from(path_str.replace(EMAIL_PLACEHOLDER, account_email.as_str()))
}

fn next_persistent_query_id(dir: &Path) -> Option<String> {
    let counter_path = dir.join(QUERY_ID_COUNTER_FILENAME);
    let current = std::fs::read_to_string(counter_path.as_path())
        .ok()
        .and_then(|text| text.trim().parse::<u64>().ok())
        .unwrap_or(0);
    let next = current.checked_add(1)?;
    std::fs::write(counter_path, format!("{next}\n")).ok()?;
    Some(next.to_string())
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

fn capture_traffic_path(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}_backend_traffic.ndjson"))
}

fn tool_usage_log_path(dir: &Path) -> PathBuf {
    dir.join("tool_usage.log")
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

pub fn backend_capture_append_event(
    session: Option<&PromptCaptureSession>,
    mut event: serde_json::Value,
) {
    let config = active_prompt_debug_http_config();
    if !config.enabled {
        return;
    }

    let Some(session) = session else {
        return;
    };

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

    let path = session.backend_traffic_path();
    if let Some(parent) = path.parent()
        && std::fs::create_dir_all(parent).is_err()
    {
        return;
    }
    append_json_line(path, &event);
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

fn call_id_from_object(
    map: &serde_json::Map<String, serde_json::Value>,
    id_field: &str,
) -> Option<String> {
    map.get(id_field)
        .or_else(|| map.get("call_id"))
        .or_else(|| map.get("id"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
}

fn register_tool_call(
    stats: &mut ToolUsageStats,
    tool_name: String,
    call_id: Option<String>,
) -> bool {
    let mut changed = false;

    if let Some(call_id) = call_id {
        stats
            .tool_names_by_call_id
            .entry(call_id.clone())
            .or_insert_with(|| tool_name.clone());
        if stats.seen_call_ids.insert(call_id) {
            *stats.counts.entry(tool_name).or_insert(0) += 1;
            changed = true;
        }
    } else {
        *stats.counts.entry(tool_name).or_insert(0) += 1;
        changed = true;
    }

    changed
}

fn register_tool_failure(
    stats: &mut ToolUsageStats,
    tool_name: Option<String>,
    call_id: Option<String>,
) -> bool {
    let resolved_tool_name = match (tool_name, call_id.as_ref()) {
        (Some(tool_name), _) => Some(tool_name),
        (None, Some(call_id)) => stats.tool_names_by_call_id.get(call_id).cloned(),
        (None, None) => None,
    };
    let Some(resolved_tool_name) = resolved_tool_name else {
        return false;
    };

    if let Some(call_id) = call_id {
        if !stats.seen_failure_call_ids.insert(call_id) {
            return false;
        }
    }

    *stats.failure_counts.entry(resolved_tool_name).or_insert(0) += 1;
    true
}

fn truncate_for_log(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }

    let mut truncated = text.chars().take(max_chars).collect::<String>();
    truncated.push_str("...");
    truncated
}

fn simplify_json_for_log(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::String(text) => serde_json::Value::String(truncate_for_log(&text, 240)),
        serde_json::Value::Array(values) => {
            serde_json::Value::Array(values.into_iter().map(simplify_json_for_log).collect())
        }
        serde_json::Value::Object(map) => serde_json::Value::Object(
            map.into_iter()
                .map(|(key, value)| (key, simplify_json_for_log(value)))
                .collect(),
        ),
        other => other,
    }
}

fn tool_usage_log_path_for_session(session: &PromptCaptureSession) -> &Path {
    session.tool_usage_log_path()
}

fn write_tool_usage_log_line(
    session: &PromptCaptureSession,
    tool_name: &str,
    payload: serde_json::Value,
) {
    append_line(
        tool_usage_log_path_for_session(session),
        &format!(
            "{} {} {} {}",
            now_unix_ms(),
            session.id(),
            tool_name,
            simplify_json_for_log(payload)
        ),
    );
}

fn tool_usage_log_payload(item: &ResponseItem) -> Option<serde_json::Value> {
    match item {
        ResponseItem::LocalShellCall {
            id,
            call_id,
            status,
            action,
        } => Some(serde_json::json!({
            "type": "local_shell_call",
            "id": id,
            "call_id": call_id,
            "status": status,
            "action": action,
        })),
        ResponseItem::FunctionCall {
            name,
            namespace,
            arguments,
            call_id,
            ..
        } => Some(serde_json::json!({
            "type": "function_call",
            "name": name,
            "namespace": namespace,
            "call_id": call_id,
            "arguments": arguments,
        })),
        ResponseItem::ToolSearchCall {
            id,
            call_id,
            status,
            execution,
            arguments,
        } => Some(serde_json::json!({
            "type": "tool_search_call",
            "id": id,
            "call_id": call_id,
            "status": status,
            "execution": execution,
            "arguments": arguments,
        })),
        ResponseItem::CustomToolCall {
            id,
            status,
            call_id,
            name,
            input,
        } => Some(serde_json::json!({
            "type": "custom_tool_call",
            "id": id,
            "status": status,
            "call_id": call_id,
            "name": name,
            "input": input,
        })),
        ResponseItem::WebSearchCall { id, status, action } => Some(serde_json::json!({
            "type": "web_search_call",
            "id": id,
            "status": status,
            "action": action,
        })),
        ResponseItem::ImageGenerationCall {
            id,
            status,
            revised_prompt,
            ..
        } => Some(serde_json::json!({
            "type": "image_generation_call",
            "id": id,
            "status": status,
            "revised_prompt": revised_prompt,
        })),
        _ => None,
    }
}

fn tool_name_from_item(item: &ResponseItem) -> String {
    match item {
        ResponseItem::LocalShellCall { .. } => "local_shell".to_string(),
        ResponseItem::FunctionCall { name, .. } => name.clone(),
        ResponseItem::ToolSearchCall { .. } => "tool_search".to_string(),
        ResponseItem::CustomToolCall { name, .. } => name.clone(),
        ResponseItem::WebSearchCall { .. } => "web_search".to_string(),
        ResponseItem::ImageGenerationCall { .. } => "image_generation".to_string(),
        _ => "tool".to_string(),
    }
}

#[cfg(test)]
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

fn record_response_item_tool_usage(
    session: Option<&PromptCaptureSession>,
    stats: &mut ToolUsageStats,
    item: &ResponseItem,
    pending_logs: &mut Vec<(String, serde_json::Value)>,
) -> bool {
    let mut log_tool_call = |tool_name: String, payload: Option<serde_json::Value>| {
        if session.is_some()
            && let Some(payload) = payload
        {
            pending_logs.push((tool_name, payload));
        }
    };

    match item {
        ResponseItem::LocalShellCall { call_id, id, .. } => {
            let changed = register_tool_call(
                stats,
                "local_shell".to_string(),
                call_id.clone().or(id.clone()),
            );
            if changed {
                log_tool_call(tool_name_from_item(item), tool_usage_log_payload(item));
            }
            changed
        }
        ResponseItem::FunctionCall { name, call_id, .. } => {
            let changed = register_tool_call(stats, name.clone(), Some(call_id.clone()));
            if changed {
                log_tool_call(tool_name_from_item(item), tool_usage_log_payload(item));
            }
            changed
        }
        ResponseItem::ToolSearchCall { call_id, .. } => {
            let changed = register_tool_call(stats, "tool_search".to_string(), call_id.clone());
            if changed {
                log_tool_call(tool_name_from_item(item), tool_usage_log_payload(item));
            }
            changed
        }
        ResponseItem::CustomToolCall { name, call_id, .. } => {
            let changed = register_tool_call(stats, name.clone(), Some(call_id.clone()));
            if changed {
                log_tool_call(tool_name_from_item(item), tool_usage_log_payload(item));
            }
            changed
        }
        ResponseItem::WebSearchCall { id, status, .. } => {
            let mut changed = register_tool_call(stats, "web_search".to_string(), id.clone());
            if changed {
                log_tool_call(tool_name_from_item(item), tool_usage_log_payload(item));
            }
            if status
                .as_deref()
                .is_some_and(|status| status != "completed")
            {
                changed |= register_tool_failure(stats, Some("web_search".to_string()), id.clone());
            }
            changed
        }
        ResponseItem::ImageGenerationCall { id, status, .. } => {
            let mut changed =
                register_tool_call(stats, "image_generation".to_string(), Some(id.clone()));
            if changed {
                log_tool_call(tool_name_from_item(item), tool_usage_log_payload(item));
            }
            if status != "completed" {
                changed |= register_tool_failure(
                    stats,
                    Some("image_generation".to_string()),
                    Some(id.clone()),
                );
            }
            changed
        }
        ResponseItem::FunctionCallOutput { call_id, output } => {
            output.success.is_some_and(|success| !success)
                && register_tool_failure(stats, None, Some(call_id.clone()))
        }
        ResponseItem::CustomToolCallOutput {
            call_id,
            name,
            output,
        } => {
            output.success.is_some_and(|success| !success)
                && register_tool_failure(stats, name.clone(), Some(call_id.clone()))
        }
        ResponseItem::ToolSearchOutput {
            call_id, status, ..
        } => {
            status != "completed"
                && register_tool_failure(stats, Some("tool_search".to_string()), call_id.clone())
        }
        _ => false,
    }
}

fn record_structured_tool_usage(
    session: Option<&PromptCaptureSession>,
    stats: &mut ToolUsageStats,
    value: &serde_json::Value,
    pending_logs: &mut Vec<(String, serde_json::Value)>,
) -> bool {
    match serde_json::from_value::<ResponseItem>(value.clone()) {
        Ok(item) => record_response_item_tool_usage(session, stats, &item, pending_logs),
        Err(_) => false,
    }
}

fn record_payload_tool_usage(
    session: Option<&PromptCaptureSession>,
    stats: &mut ToolUsageStats,
    payload: &serde_json::Value,
    pending_logs: &mut Vec<(String, serde_json::Value)>,
) -> bool {
    let mut changed = false;

    match payload {
        serde_json::Value::Array(items) => {
            for item in items {
                changed |= record_payload_tool_usage(session, stats, item, pending_logs);
            }
        }
        serde_json::Value::Object(map) => {
            if let Some(tool_name) = tool_name_from_call(map) {
                let tool_call_changed =
                    register_tool_call(stats, tool_name.clone(), call_id_from_object(map, "id"));
                if tool_call_changed && session.is_some() {
                    pending_logs.push((tool_name, simplify_json_for_log(payload.clone())));
                }
                changed |= tool_call_changed;
            }

            if let Some(item_type) = map.get("type").and_then(serde_json::Value::as_str) {
                if item_type.ends_with("_call")
                    && map
                        .get("status")
                        .and_then(serde_json::Value::as_str)
                        .is_some_and(|status| status != "completed")
                {
                    changed |= register_tool_failure(
                        stats,
                        tool_name_from_call(map),
                        call_id_from_object(map, "id"),
                    );
                }
            }

            for child in map.values() {
                changed |= record_payload_tool_usage(session, stats, child, pending_logs);
            }
        }
        _ => {}
    }

    changed
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
        "tool_failure_counts": stats.failure_counts,
        "dedupe": "call_id per process runtime",
        "note": "rough aggregate; structured response/request items first, payload fallback second; directory-scoped",
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

fn update_tool_usage_stats<F>(session: &PromptCaptureSession, mut update: F)
where
    F: FnMut(&mut ToolUsageStats) -> bool,
{
    let Some(stats_path) = stats_path_for_session(session) else {
        return;
    };

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
    let changed = update(stats);

    if changed {
        write_tool_usage_stats_file(stats_path.as_path(), stats);
    }
}

fn record_tool_usage(session: &PromptCaptureSession, payload: &serde_json::Value) {
    let mut pending_logs = Vec::new();
    update_tool_usage_stats(session, |stats| {
        if record_structured_tool_usage(Some(session), stats, payload, &mut pending_logs) {
            true
        } else {
            record_payload_tool_usage(Some(session), stats, payload, &mut pending_logs)
        }
    });
    for (tool_name, payload) in pending_logs {
        write_tool_usage_log_line(session, tool_name.as_str(), payload);
    }
}

pub fn prompt_capture_record_input_tool_usage(
    session: Option<&PromptCaptureSession>,
    items: &[ResponseItem],
) {
    let Some(session) = session else {
        return;
    };

    let mut pending_logs = Vec::new();
    update_tool_usage_stats(session, |stats| {
        let mut changed = false;
        for item in items {
            changed |=
                record_response_item_tool_usage(Some(session), stats, item, &mut pending_logs);
        }
        changed
    });
    for (tool_name, payload) in pending_logs {
        write_tool_usage_log_line(session, tool_name.as_str(), payload);
    }
}

pub fn prompt_debug_http_enabled() -> bool {
    active_prompt_debug_http_config().enabled
}

pub fn start_prompt_capture(kind: &str, input: Option<&str>) -> Option<PromptCaptureSession> {
    let config = active_prompt_debug_http_config();
    if !config.enabled && config.tool_usage_log.is_none() {
        return None;
    }

    let capture_dir = config.capture_dir.map(resolve_prompt_debug_path);
    let tool_usage_log = config.tool_usage_log.map(resolve_prompt_debug_path);
    let dir = capture_dir
        .or_else(|| {
            tool_usage_log
                .as_ref()
                .and_then(|path| path.parent().map(Path::to_path_buf))
        })
        .unwrap_or_else(default_capture_dir);
    if std::fs::create_dir_all(dir.as_path()).is_err() {
        return None;
    }
    let id = {
        let write_lock = PROMPT_DEBUG_HTTP_LOG_WRITE_LOCK.get_or_init(|| Mutex::new(()));
        let Ok(_write_guard) = write_lock.lock() else {
            return None;
        };
        next_persistent_query_id(dir.as_path())?
    };

    let input_path = dir.join(format!("{id}_input.ndjson"));
    let output_path = dir.join(format!("{id}_output.ndjson"));
    let backend_traffic_path = capture_traffic_path(dir.as_path(), id.as_str());

    // Ensure a stats file exists for this capture directory even before first tool call.
    let stats_path = dir.join("tool_usage_stats.json");
    {
        let write_lock = PROMPT_DEBUG_HTTP_LOG_WRITE_LOCK.get_or_init(|| Mutex::new(()));
        let Ok(_write_guard) = write_lock.lock() else {
            return None;
        };
        if let Ok(mut usage_guard) = tool_usage_lock().lock() {
            let stats = usage_guard
                .entry(stats_path.clone())
                .or_insert_with(ToolUsageStats::default);
            write_tool_usage_stats_file(stats_path.as_path(), stats);
        }
    }

    let tool_usage_log_file_path = tool_usage_log.unwrap_or_else(|| tool_usage_log_path(dir.as_path()));
    {
        let write_lock = PROMPT_DEBUG_HTTP_LOG_WRITE_LOCK.get_or_init(|| Mutex::new(()));
        let Ok(_write_guard) = write_lock.lock() else {
            return None;
        };
        if let Some(parent) = tool_usage_log_file_path.parent()
            && std::fs::create_dir_all(parent).is_err()
        {
            return None;
        }
        let _ = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&tool_usage_log_file_path);
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
        backend_traffic_path,
        input_path,
        output_path,
        tool_usage_log_path: tool_usage_log_file_path,
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
        let mut pending_logs = Vec::new();
        update_tool_usage_stats(session, |stats| {
            record_payload_tool_usage(Some(session), stats, &json_payload, &mut pending_logs)
        });
        for (tool_name, payload) in pending_logs {
            write_tool_usage_log_line(session, tool_name.as_str(), payload);
        }
    }
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
    let dir = resolve_prompt_debug_path(config.capture_dir.unwrap_or_else(default_capture_dir));
    Some(dir)
}

pub fn prompt_debug_http_log(message: impl AsRef<str>) {
    if !prompt_debug_http_enabled() || env::var_os(CODEX_BACKEND_CAPTURE_STDERR_ENV_VAR).is_none() {
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
        assert_eq!(
            calls,
            vec![("web_search".to_string(), Some("ws_123".to_string()))]
        );
    }

    #[test]
    fn structured_response_items_record_counts_and_failures() {
        let dir = std::env::temp_dir().join(format!(
            "codex-prompt-debug-http-structured-test-{}-{}",
            std::process::id(),
            now_unix_ms()
        ));
        std::fs::create_dir_all(dir.as_path()).expect("create temp dir");
        let session = PromptCaptureSession {
            id: "9".to_string(),
            capture_input: true,
            capture_output: true,
            backend_traffic_path: dir.join("9_backend_traffic.ndjson"),
            input_path: dir.join("9_input.ndjson"),
            output_path: dir.join("9_output.ndjson"),
            tool_usage_log_path: dir.join("tool_usage.log"),
        };

        prompt_capture_record_input_tool_usage(
            Some(&session),
            &[
                ResponseItem::FunctionCall {
                    id: None,
                    name: "exec_command".to_string(),
                    namespace: None,
                    arguments: "{}".to_string(),
                    call_id: "call_1".to_string(),
                },
                ResponseItem::FunctionCallOutput {
                    call_id: "call_1".to_string(),
                    output: codex_protocol::models::FunctionCallOutputPayload {
                        body: codex_protocol::models::FunctionCallOutputBody::Text(
                            "failed".to_string(),
                        ),
                        success: Some(false),
                    },
                },
                ResponseItem::FunctionCall {
                    id: None,
                    name: "exec_command".to_string(),
                    namespace: None,
                    arguments: "{}".to_string(),
                    call_id: "call_1".to_string(),
                },
            ],
        );

        let stats_path = stats_path_for_session(&session).expect("stats path");
        let payload = std::fs::read_to_string(stats_path).expect("read stats");
        let json: serde_json::Value = serde_json::from_str(&payload).expect("parse stats");
        assert_eq!(
            json.get("tool_counts")
                .and_then(|value| value.get("exec_command")),
            Some(&serde_json::json!(1))
        );
        assert_eq!(
            json.get("tool_failure_counts")
                .and_then(|value| value.get("exec_command")),
            Some(&serde_json::json!(1))
        );

        let log_path = tool_usage_log_path_for_session(&session);
        let log_contents = std::fs::read_to_string(log_path).expect("read tool log");
        let log_lines: Vec<&str> = log_contents.lines().collect();
        assert_eq!(log_lines.len(), 1);
        assert!(log_lines[0].contains("9"));
        assert!(log_lines[0].contains("exec_command"));

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn raw_payload_fallback_does_not_double_count_structured_call_ids() {
        let dir = std::env::temp_dir().join(format!(
            "codex-prompt-debug-http-fallback-test-{}-{}",
            std::process::id(),
            now_unix_ms()
        ));
        std::fs::create_dir_all(dir.as_path()).expect("create temp dir");
        let session = PromptCaptureSession {
            id: "10".to_string(),
            capture_input: true,
            capture_output: true,
            backend_traffic_path: dir.join("10_backend_traffic.ndjson"),
            input_path: dir.join("10_input.ndjson"),
            output_path: dir.join("10_output.ndjson"),
            tool_usage_log_path: dir.join("tool_usage.log"),
        };

        prompt_capture_record_input_tool_usage(
            Some(&session),
            &[ResponseItem::FunctionCall {
                id: None,
                name: "exec_command".to_string(),
                namespace: None,
                arguments: "{}".to_string(),
                call_id: "call_2".to_string(),
            }],
        );
        prompt_capture_append_output(
            &session,
            "responses_sse",
            r#"{"type":"function_call","name":"exec_command","call_id":"call_2"}"#,
        );

        let stats_path = stats_path_for_session(&session).expect("stats path");
        let payload = std::fs::read_to_string(stats_path).expect("read stats");
        let json: serde_json::Value = serde_json::from_str(&payload).expect("parse stats");
        assert_eq!(
            json.get("tool_counts")
                .and_then(|value| value.get("exec_command")),
            Some(&serde_json::json!(1))
        );

        let log_path = tool_usage_log_path_for_session(&session);
        let log_contents = std::fs::read_to_string(log_path).expect("read tool log");
        let log_lines: Vec<&str> = log_contents.lines().collect();
        assert_eq!(log_lines.len(), 1);
        assert!(log_lines[0].contains("10"));
        assert!(log_lines[0].contains("exec_command"));

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn resolve_capture_dir_expands_email_placeholder_when_account_email_is_available() {
        set_prompt_debug_http_account_email(Some("user@example.com".to_string()));
        let dir = resolve_prompt_debug_path(PathBuf::from("/var/tmp/prompt-debug-$EMAIL"));
        assert_eq!(dir, PathBuf::from("/var/tmp/prompt-debug-user@example.com"));
    }

    #[test]
    fn next_persistent_query_id_increments_from_counter_file() {
        let dir = std::env::temp_dir().join(format!(
            "codex-prompt-debug-http-test-{}-{}",
            std::process::id(),
            now_unix_ms()
        ));
        std::fs::create_dir_all(dir.as_path()).expect("create temp dir");
        let first = next_persistent_query_id(dir.as_path()).expect("first query id");
        let second = next_persistent_query_id(dir.as_path()).expect("second query id");
        let counter =
            std::fs::read_to_string(dir.join(QUERY_ID_COUNTER_FILENAME)).expect("counter file");
        assert_eq!(first, "1");
        assert_eq!(second, "2");
        assert_eq!(counter.trim(), "2");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn capture_traffic_path_uses_query_id_prefix() {
        let path = capture_traffic_path(Path::new("/var/tmp/codex-prompt-debug"), "7");
        assert_eq!(
            path,
            PathBuf::from("/var/tmp/codex-prompt-debug/7_backend_traffic.ndjson")
        );
    }
}
