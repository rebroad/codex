use chrono::Utc;
use std::env;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::OnceLock;
use std::sync::RwLock;

const CODEX_PROMPT_DEBUG_HTTP_ENV_VAR: &str = "CODEX_PROMPT_DEBUG_HTTP";
const CODEX_PROMPT_DEBUG_HTTP_LOGFILE_ENV_VAR: &str = "CODEX_PROMPT_DEBUG_HTTP_LOGFILE";
const CODEX_PROMPT_DEBUG_HTTP_PREFIX: &str = "[codex prompt debug]";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PromptDebugHttpConfig {
    pub enabled: bool,
    pub log_file: Option<PathBuf>,
}

static PROMPT_DEBUG_HTTP_CONFIG: OnceLock<RwLock<PromptDebugHttpConfig>> = OnceLock::new();
static PROMPT_DEBUG_HTTP_LOG_WRITE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

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
    env::var_os(CODEX_PROMPT_DEBUG_HTTP_ENV_VAR).is_some()
}

fn env_prompt_debug_http_log_file() -> Option<PathBuf> {
    env::var_os(CODEX_PROMPT_DEBUG_HTTP_LOGFILE_ENV_VAR).map(PathBuf::from)
}

fn active_prompt_debug_http_config() -> PromptDebugHttpConfig {
    let configured = configured_prompt_debug_http();
    let env_enabled = env_prompt_debug_http_enabled();
    let enabled = env_enabled || configured.enabled;
    let log_file = env_prompt_debug_http_log_file().or(configured.log_file);
    PromptDebugHttpConfig { enabled, log_file }
}

pub fn prompt_debug_http_enabled() -> bool {
    active_prompt_debug_http_config().enabled
}

pub fn prompt_debug_http_log(message: impl AsRef<str>) {
    let config = active_prompt_debug_http_config();
    if !config.enabled {
        return;
    }

    let now = Utc::now();
    let timestamp = now.format("%Y-%m-%dT%H:%M:%S%.2f");
    let message = message.as_ref();
    let line = format!("{CODEX_PROMPT_DEBUG_HTTP_PREFIX} [{timestamp}] {message}");
    if let Some(path) = config.log_file {
        let write_lock = PROMPT_DEBUG_HTTP_LOG_WRITE_LOCK.get_or_init(|| Mutex::new(()));
        let Ok(_guard) = write_lock.lock() else {
            eprintln!("{line}");
            return;
        };

        if let Some(parent) = path.parent()
            && let Err(err) = std::fs::create_dir_all(parent)
        {
            eprintln!("{line}");
            eprintln!(
                "{CODEX_PROMPT_DEBUG_HTTP_PREFIX} failed to create debug log directory {}: {err}",
                parent.display(),
            );
            return;
        }

        match OpenOptions::new().create(true).append(true).open(&path) {
            Ok(mut file) => {
                if let Err(err) = writeln!(file, "{line}") {
                    eprintln!("{line}");
                    eprintln!(
                        "{CODEX_PROMPT_DEBUG_HTTP_PREFIX} failed to write debug log file {}: {err}",
                        path.display(),
                    );
                }
            }
            Err(err) => {
                eprintln!("{line}");
                eprintln!(
                    "{CODEX_PROMPT_DEBUG_HTTP_PREFIX} failed to open debug log file {}: {err}",
                    path.display(),
                );
            }
        }
    } else {
        eprintln!("{line}");
    }
}
