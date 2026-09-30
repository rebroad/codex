//! Resolve saved-session state needed before resuming or forking a thread.
//!
//! The app-server API owns thread metadata. This module coordinates the TUI-specific
//! cwd prompt using the cwd reported by the selected thread.

use std::path::Path;
use std::path::PathBuf;

use crate::app_server_session::AppServerSession;
use crate::cwd_prompt;
use crate::cwd_prompt::CwdPromptAction;
use crate::cwd_prompt::CwdPromptOutcome;
use crate::legacy_core::config::Config;
use crate::resume_picker::SessionTarget;
use crate::tui::Tui;
use codex_config::types::ResumeCwdMode;
use codex_protocol::ThreadId;
use codex_protocol::protocol::SessionMetaLine;
use codex_rollout::RolloutItem;
use codex_rollout::builder_from_items;
use codex_rollout::read_session_meta_line;
use codex_state::StateRuntime;
use codex_utils_path as path_utils;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ResolveCwdOutcome {
    Continue(Option<PathBuf>),
    /// An interactive prompt was shown, so previously cached authentication may be stale.
    ContinueAfterPrompt(PathBuf),
    Exit,
}

pub(crate) struct ResumeCwdContext<'path> {
    pub(crate) target_session: Option<&'path SessionTarget>,
    pub(crate) state_db_ctx: Option<&'path StateRuntime>,
    pub(crate) current_cwd: &'path Path,
    pub(crate) remembered_current_cwd: &'path Path,
    pub(crate) allow_remember_current: bool,
    pub(crate) mode: Option<ResumeCwdMode>,
}

pub(crate) fn effective_resume_cwd_mode(
    configured_mode: Option<ResumeCwdMode>,
    cwd_override: Option<&Path>,
) -> Option<ResumeCwdMode> {
    if cwd_override.is_some() {
        Some(ResumeCwdMode::Current)
    } else {
        configured_mode
    }
}

pub(crate) async fn read_session_cwd(
    app_server: &mut AppServerSession,
    thread_id: ThreadId,
) -> Option<PathBuf> {
    match app_server
        .thread_read(thread_id, /*include_turns*/ false)
        .await
    {
        Ok(thread) => Some(thread.cwd.to_path_buf()),
        Err(err) => {
            tracing::warn!(%thread_id, %err, "Failed to read session cwd from app server");
            None
        }
    }
}

pub(crate) async fn resolve_cwd_for_resume_or_fork(
    tui: &mut Tui,
    config: &Config,
    history_cwd: Option<PathBuf>,
    action: CwdPromptAction,
    cwd_context: ResumeCwdContext<'_>,
) -> color_eyre::Result<ResolveCwdOutcome> {
    if matches!(cwd_context.mode, Some(ResumeCwdMode::Current)) {
        return Ok(ResolveCwdOutcome::Continue(Some(
            cwd_context.remembered_current_cwd.to_path_buf(),
        )));
    }
    let Some(history_cwd) = history_cwd else {
        if matches!(cwd_context.mode, Some(ResumeCwdMode::Session)) {
            color_eyre::eyre::bail!(
                "failed to determine the working directory recorded for the selected session"
            );
        }
        return Ok(ResolveCwdOutcome::Continue(None));
    };
    match cwd_context.mode {
        Some(ResumeCwdMode::Session) => {
            return Ok(ResolveCwdOutcome::Continue(Some(history_cwd)));
        }
        Some(ResumeCwdMode::Current) | None => {}
    }
    if cwds_differ(cwd_context.current_cwd, &history_cwd) {
        let selection_outcome = cwd_prompt::run_cwd_selection_prompt(
            tui,
            config,
            action,
            cwd_context.current_cwd,
            &history_cwd,
            cwd_context.remembered_current_cwd,
            cwd_context.allow_remember_current,
        )
        .await?;
        return Ok(match selection_outcome {
            CwdPromptOutcome::Selection(selection) => {
                let selected_cwd = selection
                    .selected_cwd(
                        cwd_context.current_cwd,
                        &history_cwd,
                        cwd_context.remembered_current_cwd,
                    )
                    .to_path_buf();
                if selection == cwd_prompt::CwdSelection::CurrentAndRememberForSession {
                    let Some(target_session) = cwd_context.target_session else {
                        color_eyre::eyre::bail!(
                            "cannot update the session directory without a selected session"
                        );
                    };
                    persist_session_cwd(
                        cwd_context.state_db_ctx,
                        target_session,
                        selected_cwd.as_path(),
                    )
                    .await?;
                }
                ResolveCwdOutcome::ContinueAfterPrompt(selected_cwd)
            }
            CwdPromptOutcome::Exit => ResolveCwdOutcome::Exit,
        });
    }
    Ok(ResolveCwdOutcome::Continue(Some(history_cwd)))
}

async fn persist_session_cwd(
    state_db_ctx: Option<&StateRuntime>,
    target_session: &SessionTarget,
    cwd: &Path,
) -> color_eyre::Result<()> {
    let Some(rollout_path) = target_session.path.as_deref() else {
        color_eyre::eyre::bail!("cannot update the session directory without a local rollout path");
    };
    let mut session_meta = read_session_meta_line(rollout_path).await?;
    session_meta.meta.cwd = cwd.to_path_buf();
    let item = RolloutItem::SessionMeta(SessionMetaLine {
        meta: session_meta.meta,
        git: session_meta.git,
    });

    if let Some(state_db_ctx) = state_db_ctx {
        let builder = builder_from_items(std::slice::from_ref(&item), rollout_path)
            .ok_or_else(|| color_eyre::eyre::eyre!("session metadata has no thread ID"))?;
        state_db_ctx
            .apply_rollout_items(&builder, std::slice::from_ref(&item), None, None)
            .await
            .map_err(|err| color_eyre::eyre::eyre!(err))?;
    }
    Ok(())
}

pub(crate) fn cwds_differ(current_cwd: &Path, session_cwd: &Path) -> bool {
    !path_utils::paths_match_after_normalization(current_cwd, session_cwd)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    #[tokio::test]
    async fn configured_resume_cwd_skips_prompt() -> color_eyre::Result<()> {
        let temp_dir = TempDir::new()?;
        let session_cwd = temp_dir.path().join("session");
        let config = crate::legacy_core::config::ConfigBuilder::default()
            .codex_home(temp_dir.path().to_path_buf())
            .build()
            .await?;
        let current_cwd = config.cwd.to_path_buf();
        let mut tui = crate::tui::test_support::make_test_tui()?;

        for (cwd_mode, expected_cwd) in [
            (ResumeCwdMode::Current, current_cwd.clone()),
            (ResumeCwdMode::Session, session_cwd.clone()),
        ] {
            let outcome = resolve_cwd_for_resume_or_fork(
                &mut tui,
                &config,
                Some(session_cwd.clone()),
                CwdPromptAction::Fork,
                ResumeCwdContext {
                    target_session: None,
                    state_db_ctx: None,
                    current_cwd: &current_cwd,
                    remembered_current_cwd: &current_cwd,
                    allow_remember_current: true,
                    mode: Some(cwd_mode),
                },
            )
            .await?;

            assert_eq!(outcome, ResolveCwdOutcome::Continue(Some(expected_cwd)));
        }
        Ok(())
    }

    #[tokio::test]
    async fn matching_resume_cwd_skips_prompt_without_configured_mode() -> color_eyre::Result<()> {
        let temp_dir = TempDir::new()?;
        let config = crate::legacy_core::config::ConfigBuilder::default()
            .codex_home(temp_dir.path().to_path_buf())
            .build()
            .await?;
        let current_cwd = config.cwd.to_path_buf();
        let mut tui = crate::tui::test_support::make_test_tui()?;

        let outcome = resolve_cwd_for_resume_or_fork(
            &mut tui,
            &config,
            Some(current_cwd.clone()),
            CwdPromptAction::Resume,
            ResumeCwdContext {
                target_session: None,
                state_db_ctx: None,
                current_cwd: &current_cwd,
                remembered_current_cwd: &current_cwd,
                allow_remember_current: true,
                mode: None,
            },
        )
        .await?;

        assert_eq!(outcome, ResolveCwdOutcome::Continue(Some(current_cwd)));
        Ok(())
    }

    #[tokio::test]
    async fn configured_session_cwd_rejects_missing_metadata() -> color_eyre::Result<()> {
        let temp_dir = TempDir::new()?;
        let config = crate::legacy_core::config::ConfigBuilder::default()
            .codex_home(temp_dir.path().to_path_buf())
            .build()
            .await?;
        let current_cwd = config.cwd.to_path_buf();
        let mut tui = crate::tui::test_support::make_test_tui()?;

        let error = resolve_cwd_for_resume_or_fork(
            &mut tui,
            &config,
            /*history_cwd*/ None,
            CwdPromptAction::Resume,
            ResumeCwdContext {
                target_session: None,
                state_db_ctx: None,
                current_cwd: &current_cwd,
                remembered_current_cwd: &current_cwd,
                allow_remember_current: true,
                mode: Some(ResumeCwdMode::Session),
            },
        )
        .await
        .expect_err("session mode should reject unavailable metadata");

        assert_eq!(
            error.to_string(),
            "failed to determine the working directory recorded for the selected session"
        );
        Ok(())
    }
}
