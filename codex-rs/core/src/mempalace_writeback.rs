use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use chrono::Utc;
use codex_protocol::ThreadId;
use serde_json::json;
use tracing::warn;

use crate::codex::Session;

const MEMPALACE_SERVER: &str = "mempalace";
const MEMPALACE_WING: &str = "sessions";
const DEFAULT_ROOM: &str = "decisions";
const PREFERENCE_ROOM: &str = "preferences";
const FACT_ROOM: &str = "project_facts";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MemoryKind {
    Decision,
    Rationale,
    Preference,
    ProjectFact,
}

impl MemoryKind {
    fn room(self) -> &'static str {
        match self {
            Self::Decision | Self::Rationale => DEFAULT_ROOM,
            Self::Preference => PREFERENCE_ROOM,
            Self::ProjectFact => FACT_ROOM,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Decision => "decision",
            Self::Rationale => "rationale",
            Self::Preference => "preference",
            Self::ProjectFact => "project_fact",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MemoryCandidate {
    kind: MemoryKind,
    summary: String,
    rationale: Option<String>,
    preference_value: Option<String>,
}

pub(crate) fn spawn_after_agent_writeback(
    session: Arc<Session>,
    thread_id: ThreadId,
    turn_id: String,
    cwd: PathBuf,
    input_messages: Vec<String>,
    last_assistant_message: Option<String>,
) {
    tokio::spawn(async move {
        let turn_id_for_log = turn_id.clone();
        if let Err(error) = run_after_agent_writeback(
            session,
            thread_id,
            turn_id,
            cwd,
            input_messages,
            last_assistant_message,
        )
        .await
        {
            warn!(
                thread_id = %thread_id,
                turn_id = %turn_id_for_log,
                error = %error,
                "MemPalace automatic write-back failed"
            );
        }
    });
}

async fn run_after_agent_writeback(
    session: Arc<Session>,
    thread_id: ThreadId,
    turn_id: String,
    cwd: PathBuf,
    input_messages: Vec<String>,
    last_assistant_message: Option<String>,
) -> Result<()> {
    if !mempalace_tools_visible(session.as_ref()).await? {
        return Ok(());
    }

    let Some(candidate) = classify_turn(&input_messages, last_assistant_message.as_deref())
    else {
        return Ok(());
    };

    let writeback_id = format!("codex://{thread_id}/{turn_id}");
    if drawer_already_exists(session.as_ref(), &writeback_id).await? {
        return Ok(());
    }

    let drawer_content = build_drawer_content(
        &writeback_id,
        thread_id,
        &turn_id,
        &cwd,
        &input_messages,
        last_assistant_message.as_deref(),
        &candidate,
    );
    let args = json!({
        "added_by": "codex",
        "content": drawer_content,
        "room": candidate.kind.room(),
        "source_file": cwd.to_string_lossy().to_string(),
        "wing": MEMPALACE_WING,
    });

    let result = session
        .call_tool(MEMPALACE_SERVER, "mempalace_add_drawer", Some(args), None)
        .await?;
    let output = result
        .as_function_call_output_payload()
        .body
        .to_text()
        .unwrap_or_default();
    tracing::debug!(
        thread_id = %thread_id,
        turn_id = %turn_id,
        kind = candidate.kind.label(),
        output = %truncate_text(&output, 200),
        "MemPalace write-back stored"
    );

    if matches!(candidate.kind, MemoryKind::Preference)
        && let Some(preference_value) = candidate.preference_value.as_deref()
        && let Err(error) = write_preference_fact(session.as_ref(), preference_value).await
    {
        warn!(
            thread_id = %thread_id,
            turn_id = %turn_id,
            error = %error,
            "MemPalace preference fact write failed"
        );
    }

    Ok(())
}

async fn mempalace_tools_visible(session: &Session) -> Result<bool> {
    let tools = session
        .services
        .mcp_connection_manager
        .read()
        .await
        .list_all_tools()
        .await;
    Ok(tools.keys().any(|name| name.starts_with("mcp__mempalace__")))
}

async fn drawer_already_exists(session: &Session, writeback_id: &str) -> Result<bool> {
    let args = json!({
        "query": format!("writeback_id {writeback_id}"),
        "include_decayed": false,
        "limit": 5,
        "max_distance": 0.35,
    });
    let result = session
        .call_tool(MEMPALACE_SERVER, "mempalace_search", Some(args), None)
        .await?;
    let output = result
        .as_function_call_output_payload()
        .body
        .to_text()
        .unwrap_or_default();
    Ok(output.contains(writeback_id))
}

async fn write_preference_fact(session: &Session, preference_value: &str) -> Result<()> {
    let args = json!({
        "subject": "Codex",
        "predicate": "prefers",
        "object": truncate_text(preference_value, 120),
        "valid_from": Utc::now().date_naive().format("%Y-%m-%d").to_string(),
    });
    session
        .call_tool(MEMPALACE_SERVER, "mempalace_kg_add", Some(args), None)
        .await?;
    Ok(())
}

pub(crate) fn classify_turn(
    input_messages: &[String],
    last_assistant_message: Option<&str>,
) -> Option<MemoryCandidate> {
    let assistant = normalize_text(last_assistant_message?);
    let user_context = normalize_text(&input_messages.join("\n"));
    let combined = format!("{user_context}\n{assistant}");

    if is_noise(&assistant) {
        return None;
    }

    if let Some(preference_value) = extract_preference_value(&assistant) {
        return Some(MemoryCandidate {
            kind: MemoryKind::Preference,
            summary: summarize(&assistant),
            rationale: extract_rationale_snippet(&assistant),
            preference_value: Some(preference_value),
        });
    }

    if contains_any(&combined, &["because", "so that", "in order to", "therefore", "rationale"])
        || contains_any(&assistant, &["why", "reason", "tradeoff"])
    {
        return Some(MemoryCandidate {
            kind: MemoryKind::Rationale,
            summary: summarize(&assistant),
            rationale: extract_rationale_snippet(&assistant),
            preference_value: None,
        });
    }

    if contains_any(
        &combined,
        &[
            "we should",
            "let's",
            "decided",
            "recommend",
            "suggest",
            "keep ",
            "switch ",
            "replace ",
            "move ",
            "use ",
            "remove ",
            "add ",
            "implement",
            "change",
        ],
    ) {
        return Some(MemoryCandidate {
            kind: MemoryKind::Decision,
            summary: summarize(&assistant),
            rationale: extract_rationale_snippet(&assistant),
            preference_value: None,
        });
    }

    if contains_any(
        &combined,
        &[
            "project", "code", "module", "prompt", "hook", "config", "build tree", "source tree",
            "wiring", "workflow", "pipeline", "write-back",
        ],
    ) && word_count(&assistant) >= 10
    {
        return Some(MemoryCandidate {
            kind: MemoryKind::ProjectFact,
            summary: summarize(&assistant),
            rationale: extract_rationale_snippet(&assistant),
            preference_value: None,
        });
    }

    None
}

pub(crate) fn build_drawer_content(
    writeback_id: &str,
    thread_id: ThreadId,
    turn_id: &str,
    cwd: &PathBuf,
    input_messages: &[String],
    last_assistant_message: Option<&str>,
    candidate: &MemoryCandidate,
) -> String {
    let captured_at = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let user_excerpt = truncate_text(&normalize_text(&input_messages.join("\n")), 350);
    let assistant_excerpt = truncate_text(&normalize_text(last_assistant_message.unwrap_or("")), 500);
    let rationale = candidate
        .rationale
        .as_ref()
        .map(|text| format!("rationale: {}", truncate_text(text, 250)))
        .unwrap_or_default();
    let preference_value = candidate
        .preference_value
        .as_ref()
        .map(|text| format!("preference_value: {}", truncate_text(text, 250)))
        .unwrap_or_default();

    format!(
        "\
# MemPalace turn memory\n\
writeback_id: {writeback_id}\n\
kind: {}\n\
session_id: {thread_id}\n\
turn_id: {turn_id}\n\
captured_at: {captured_at}\n\
cwd: {}\n\
summary: {}\n\
{}\n\
{}\n\
evidence:\n\
- user: {user_excerpt}\n\
- assistant: {assistant_excerpt}\n",
        candidate.kind.label(),
        cwd.display(),
        truncate_text(&candidate.summary, 350),
        rationale,
        preference_value
    )
}

fn normalize_text(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ").trim().to_string()
}

fn truncate_text(text: &str, max_chars: usize) -> String {
    text.chars().take(max_chars).collect()
}

fn word_count(text: &str) -> usize {
    text.split_whitespace().count()
}

fn contains_any(text: &str, needles: &[&str]) -> bool {
    let lower = text.to_lowercase();
    needles.iter().any(|needle| lower.contains(needle))
}

fn is_noise(text: &str) -> bool {
    let lower = text.to_lowercase();
    if word_count(text) <= 3 {
        return lower == "ok"
            || lower == "okay"
            || lower == "thanks"
            || lower == "thank you"
            || lower == "sure"
            || lower == "sounds good"
            || lower == "got it"
            || lower == "done";
    }

    contains_any(
        &lower,
        &[
            "happy to help",
            "let me know",
            "if you want",
            "anything else",
            "glad to help",
        ],
    ) && word_count(text) < 12
}

fn summarize(text: &str) -> String {
    let trimmed = normalize_text(text);
    if let Some((first, _rest)) = split_first_sentence(&trimmed) {
        truncate_text(first, 220)
    } else {
        truncate_text(&trimmed, 220)
    }
}

fn split_first_sentence(text: &str) -> Option<(&str, &str)> {
    for (idx, ch) in text.char_indices() {
        if matches!(ch, '.' | '!' | '?') {
            let head = text[..=idx].trim();
            let tail = text[idx + ch.len_utf8()..].trim();
            return Some((head, tail));
        }
    }
    None
}

fn extract_rationale_snippet(text: &str) -> Option<String> {
    let normalized = normalize_text(text);
    let lower = normalized.to_lowercase();
    for cue in ["because", "so that", "in order to", "therefore", "due to"] {
        if let Some(idx) = lower.find(cue) {
            let snippet = normalized[idx..].trim();
            return Some(truncate_text(snippet, 250));
        }
    }
    None
}

fn extract_preference_value(text: &str) -> Option<String> {
    let normalized = normalize_text(text);
    let lower = normalized.to_lowercase();

    for cue in [
        "i prefer ",
        "prefer ",
        "i like ",
        "please use ",
        "don't use ",
        "do not use ",
        "keep using ",
        "stick with ",
        "avoid ",
    ] {
        if let Some(idx) = lower.find(cue) {
            let value = normalized[idx + cue.len()..]
                .trim()
                .trim_matches(|ch: char| matches!(ch, '.' | ',' | ';' | ':' | '`'));
            if !value.is_empty() {
                return Some(truncate_text(value, 120));
            }
        }
    }

    if lower.starts_with("use ") {
        let value = normalized["use ".len()..]
            .trim()
            .trim_matches(|ch: char| matches!(ch, '.' | ',' | ';' | ':' | '`'));
        if !value.is_empty() {
            return Some(truncate_text(value, 120));
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn candidate_summary(text: &str) -> Option<MemoryCandidate> {
        classify_turn(&["User context".to_string()], Some(text))
    }

    #[test]
    fn classify_turn_detects_decision() {
        let candidate = candidate_summary("We should keep MemPalace and add automatic write-back in code.")
            .expect("expected memory candidate");
        assert_eq!(candidate.kind, MemoryKind::Decision);
        assert!(candidate.summary.contains("We should keep MemPalace"));
    }

    #[test]
    fn classify_turn_ignores_transient_acknowledgement() {
        assert!(candidate_summary("Sounds good, thanks!").is_none());
    }

    #[test]
    fn classify_turn_detects_rationale() {
        let candidate = candidate_summary(
            "We should keep the MCP because prompt-only memory is not reliable.",
        )
        .expect("expected memory candidate");
        assert_eq!(candidate.kind, MemoryKind::Rationale);
        assert!(candidate
            .rationale
            .as_deref()
            .unwrap_or_default()
            .contains("because prompt-only memory"));
    }

    #[test]
    fn classify_turn_detects_preference() {
        let candidate = candidate_summary("I prefer to use rg for source search.")
            .expect("expected memory candidate");
        assert_eq!(candidate.kind, MemoryKind::Preference);
        assert_eq!(candidate.preference_value.as_deref(), Some("to use rg for source search"));
    }

    #[test]
    fn build_drawer_content_includes_marker_and_summary() {
        let candidate = MemoryCandidate {
            kind: MemoryKind::Decision,
            summary: "Keep MemPalace and add automatic write-back.".to_string(),
            rationale: Some("because prompt-only memory is not reliable".to_string()),
            preference_value: None,
        };
        let drawer = build_drawer_content(
            "codex://thread/turn",
            ThreadId::default(),
            "turn",
            &PathBuf::from("/work"),
            &["User context".to_string()],
            Some("We should keep MemPalace."),
            &candidate,
        );
        assert!(drawer.contains("writeback_id: codex://thread/turn"));
        assert!(drawer.contains("kind: decision"));
        assert!(drawer.contains("summary: Keep MemPalace and add automatic write-back."));
        assert!(drawer.contains("rationale: because prompt-only memory is not reliable"));
    }
}
