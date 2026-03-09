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

const CODEX_BACKEND_CAPTURE_ENV_VAR: &str = "CODEX_BACKEND_CAPTURE";
const CODEX_BACKEND_CAPTURE_INPUT_ENV_VAR: &str = "CODEX_BACKEND_CAPTURE_INPUT";
const CODEX_BACKEND_CAPTURE_OUTPUT_ENV_VAR: &str = "CODEX_BACKEND_CAPTURE_OUTPUT";
const CODEX_BACKEND_CAPTURE_REASONING_ENV_VAR: &str = "CODEX_BACKEND_CAPTURE_REASONING";
const CODEX_BACKEND_CAPTURE_DIR_ENV_VAR: &str = "CODEX_BACKEND_CAPTURE_DIR";
const CODEX_PROMPT_DEBUG_HTTP_PREFIX: &str = "[codex backend capture]";

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
static CAPTURE_COUNTER: AtomicU64 = AtomicU64::new(1);

fn config_lock() -> &'static RwLock<PromptDebugHttpConfig> {
    PROMPT_DEBUG_HTTP_CONFIG.get_or_init(|| RwLock::new(PromptDebugHttpConfig::default()))
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
