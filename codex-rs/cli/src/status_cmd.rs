use std::path::PathBuf;

use anyhow::Context;
use codex_core::INTERACTIVE_SESSION_SOURCES;
use codex_core::RolloutRecorder;
use codex_core::ThreadItem;
use codex_core::ThreadSortKey;
use codex_core::config::Config;
use codex_protocol::protocol::TokenUsage;
use codex_state::read_rollout_thread_snapshot;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadStatusSelector {
    pub thread_id: Option<String>,
    pub last: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadStatusOutputFormat {
    Human,
    Json,
    Telegram,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadStatusSnapshot {
    pub thread_id: Option<String>,
    pub rollout_path: PathBuf,
    pub model: Option<String>,
    pub cwd: Option<PathBuf>,
    pub updated_at: Option<String>,
    pub preview: Option<String>,
    pub model_context_window: Option<i64>,
    pub total_tokens: i64,
    pub input_tokens: i64,
    pub cached_input_tokens: i64,
    pub output_tokens: i64,
    pub reasoning_output_tokens: i64,
    pub remaining_tokens: Option<i64>,
    pub remaining_percent: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DerivedContextUsage {
    pub total_tokens: i64,
    pub input_tokens: i64,
    pub cached_input_tokens: i64,
    pub output_tokens: i64,
    pub reasoning_output_tokens: i64,
    pub remaining_tokens: Option<i64>,
    pub remaining_percent: Option<i64>,
}

pub fn derive_context_usage(
    usage: &TokenUsage,
    model_context_window: Option<i64>,
) -> DerivedContextUsage {
    let remaining_tokens =
        model_context_window.map(|context_window| (context_window - usage.total_tokens).max(0));
    let remaining_percent =
        model_context_window.map(|context_window| usage.percent_of_context_window_remaining(context_window));
    DerivedContextUsage {
        total_tokens: usage.total_tokens,
        input_tokens: usage.input_tokens,
        cached_input_tokens: usage.cached_input_tokens,
        output_tokens: usage.output_tokens,
        reasoning_output_tokens: usage.reasoning_output_tokens,
        remaining_tokens,
        remaining_percent,
    }
}

pub async fn load_thread_status_snapshot(
    config: &Config,
    selector: &ThreadStatusSelector,
) -> anyhow::Result<ThreadStatusSnapshot> {
    let thread_item = select_thread_item(config, selector).await?;
    let thread_id = thread_item.thread_id.as_ref().map(ToString::to_string);
    let snapshot = read_rollout_thread_snapshot(thread_item.path.as_path())
        .await
        .with_context(|| {
            let id = thread_id.as_deref().unwrap_or("<unknown>");
            format!("failed to read rollout snapshot for thread {id}")
        })?;
    let model = snapshot
        .latest_turn_context
        .as_ref()
        .map(|item| item.model.clone());
    let model_context_window = snapshot
        .latest_token_usage_info
        .as_ref()
        .and_then(|info| info.model_context_window);
    let usage = snapshot
        .latest_token_usage_info
        .as_ref()
        .map(|info| info.total_token_usage.clone())
        .unwrap_or_default();
    let derived = derive_context_usage(&usage, model_context_window);

    Ok(ThreadStatusSnapshot {
        thread_id,
        rollout_path: thread_item.path,
        model,
        cwd: snapshot.latest_turn_context.map(|item| item.cwd).or(thread_item.cwd),
        updated_at: thread_item.updated_at,
        preview: thread_item.first_user_message,
        model_context_window,
        total_tokens: derived.total_tokens,
        input_tokens: derived.input_tokens,
        cached_input_tokens: derived.cached_input_tokens,
        output_tokens: derived.output_tokens,
        reasoning_output_tokens: derived.reasoning_output_tokens,
        remaining_tokens: derived.remaining_tokens,
        remaining_percent: derived.remaining_percent,
    })
}

pub fn render_thread_status(
    snapshot: &ThreadStatusSnapshot,
    format: ThreadStatusOutputFormat,
) -> anyhow::Result<Vec<String>> {
    match format {
        ThreadStatusOutputFormat::Human => Ok(render_human_thread_status(snapshot)),
        ThreadStatusOutputFormat::Telegram => Ok(render_telegram_thread_status(snapshot)),
        ThreadStatusOutputFormat::Json => Ok(vec![render_json_thread_status(snapshot)?]),
    }
}

fn render_json_thread_status(snapshot: &ThreadStatusSnapshot) -> anyhow::Result<String> {
    serde_json::to_string_pretty(&serde_json::json!({
        "thread_id": snapshot.thread_id,
        "rollout_path": snapshot.rollout_path,
        "model": snapshot.model,
        "cwd": snapshot.cwd,
        "updated_at": snapshot.updated_at,
        "preview": snapshot.preview,
        "model_context_window": snapshot.model_context_window,
        "total_tokens": snapshot.total_tokens,
        "input_tokens": snapshot.input_tokens,
        "cached_input_tokens": snapshot.cached_input_tokens,
        "output_tokens": snapshot.output_tokens,
        "reasoning_output_tokens": snapshot.reasoning_output_tokens,
        "remaining_tokens": snapshot.remaining_tokens,
        "remaining_percent": snapshot.remaining_percent,
    }))
    .map_err(Into::into)
}

async fn select_thread_item(
    config: &Config,
    selector: &ThreadStatusSelector,
) -> anyhow::Result<ThreadItem> {
    if selector.last {
        let page = RolloutRecorder::list_threads(
            config,
            /*page_size*/ 1,
            None,
            ThreadSortKey::UpdatedAt,
            INTERACTIVE_SESSION_SOURCES.as_slice(),
            /*model_providers*/ None,
            &config.model_provider_id,
            /*search_term*/ None,
        )
        .await?;
        return page
            .items
            .into_iter()
            .next()
            .context("no interactive threads found");
    }

    let requested = selector
        .thread_id
        .as_deref()
        .context("missing thread selector")?;
    let mut cursor = None;
    loop {
        let page = RolloutRecorder::list_threads(
            config,
            /*page_size*/ 100,
            cursor.as_ref(),
            ThreadSortKey::UpdatedAt,
            INTERACTIVE_SESSION_SOURCES.as_slice(),
            /*model_providers*/ None,
            &config.model_provider_id,
            /*search_term*/ None,
        )
        .await?;
        if let Some(item) = page
            .items
            .into_iter()
            .find(|item| item.thread_id.as_ref().is_some_and(|id| id.to_string() == requested))
        {
            return Ok(item);
        }
        if page.next_cursor.is_none() {
            break;
        }
        cursor = page.next_cursor;
    }
    anyhow::bail!("thread not found: {requested}")
}

fn render_human_thread_status(snapshot: &ThreadStatusSnapshot) -> Vec<String> {
    let mut lines = Vec::new();
    lines.push(format!(
        "Thread: {}",
        snapshot.thread_id.as_deref().unwrap_or("<unknown>")
    ));
    if let Some(model) = snapshot.model.as_deref() {
        lines.push(format!("Model: {model}"));
    }
    if let Some(cwd) = snapshot.cwd.as_ref() {
        lines.push(format!("Cwd: {}", cwd.display()));
    }
    if let Some(updated_at) = snapshot.updated_at.as_deref() {
        lines.push(format!("Updated: {updated_at}"));
    }
    if let Some(preview) = snapshot.preview.as_deref() {
        lines.push(format!("Preview: {preview}"));
    }
    lines.push(format!("Rollout: {}", snapshot.rollout_path.display()));
    lines.extend(render_usage_lines(snapshot));
    lines
}

fn render_telegram_thread_status(snapshot: &ThreadStatusSnapshot) -> Vec<String> {
    let thread = snapshot.thread_id.as_deref().unwrap_or("<unknown>");
    let model = snapshot.model.as_deref().unwrap_or("<unknown>");
    let used = snapshot.total_tokens;
    let remaining = snapshot
        .remaining_tokens
        .map(|value| value.to_string())
        .unwrap_or_else(|| "n/a".to_string());
    let remaining_pct = snapshot
        .remaining_percent
        .map(|value| format!("{value}%"))
        .unwrap_or_else(|| "n/a".to_string());
    vec![
        format!("thread {thread}"),
        format!("model {model}"),
        format!("used {used}"),
        format!("remaining {remaining} ({remaining_pct})"),
        format!(
            "input {} cached {} output {} reasoning {}",
            snapshot.input_tokens,
            snapshot.cached_input_tokens,
            snapshot.output_tokens,
            snapshot.reasoning_output_tokens
        ),
    ]
}

fn render_usage_lines(snapshot: &ThreadStatusSnapshot) -> Vec<String> {
    let mut lines = Vec::new();
    let context_window = snapshot
        .model_context_window
        .map(|value| value.to_string())
        .unwrap_or_else(|| "n/a".to_string());
    let remaining = snapshot
        .remaining_tokens
        .map(|value| value.to_string())
        .unwrap_or_else(|| "n/a".to_string());
    let remaining_pct = snapshot
        .remaining_percent
        .map(|value| format!("{value}%"))
        .unwrap_or_else(|| "n/a".to_string());
    lines.push(format!("Context window: {context_window}"));
    lines.push(format!("Used tokens: {}", snapshot.total_tokens));
    lines.push(format!("Remaining tokens: {remaining}"));
    lines.push(format!("Remaining percent: {remaining_pct}"));
    lines.push(format!("Input tokens: {}", snapshot.input_tokens));
    lines.push(format!("Cached input tokens: {}", snapshot.cached_input_tokens));
    lines.push(format!("Output tokens: {}", snapshot.output_tokens));
    lines.push(format!(
        "Reasoning output tokens: {}",
        snapshot.reasoning_output_tokens
    ));
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_context_usage_counts_cached_tokens_once() {
        let usage = TokenUsage {
            input_tokens: 100,
            cached_input_tokens: 25,
            output_tokens: 40,
            reasoning_output_tokens: 10,
            total_tokens: 140,
        };
        let derived = derive_context_usage(&usage, Some(1_000));
        assert_eq!(
            derived,
            DerivedContextUsage {
                total_tokens: 140,
                input_tokens: 100,
                cached_input_tokens: 25,
                output_tokens: 40,
                reasoning_output_tokens: 10,
                remaining_tokens: Some(860),
                remaining_percent: Some(86),
            }
        );
    }
}
