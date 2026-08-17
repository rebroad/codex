//! Resume history policy for local rollouts that may migrate between formats.

use super::AppServerSession;
use super::AppServerStartedThread;
use super::HistoryHydrationScope;
use super::ResumeModelSettings;
use super::ThreadHistorySupport;
use super::bootstrap_request_error;
use super::is_active_writer_conflict;
use super::is_history_pagination_unsupported;
use super::started_thread_from_resume_response;
use super::thread_resume_params_from_config;
use crate::legacy_core::config::Config;
use codex_app_server_client::TypedRequestError;
use codex_app_server_protocol::ClientRequest;
use codex_app_server_protocol::ThreadHistoryMode;
use codex_app_server_protocol::ThreadResumeResponse;
use codex_features::Feature;
use codex_protocol::ThreadId;
use color_eyre::eyre::Result;
use color_eyre::eyre::WrapErr;

impl AppServerSession {
    /// Captures the server's startup migration policy before workspace config can change.
    ///
    /// Sessions without a recorded startup config conservatively assume migration is enabled.
    pub(crate) fn with_startup_config(mut self, config: &Config) -> Self {
        self.background_rollout_migration_enabled = config
            .features
            .enabled(Feature::BackgroundPaginatedRolloutMigration);
        self.task_tool_capabilities_dir = (!self.uses_embedded_app_server())
            .then(|| config.codex_home.join("tui-thread-reference-capabilities"));
        self
    }

    /// Retains trusted embedded-server history metadata for a later resume.
    ///
    /// Remote hints are ignored because another process can migrate the thread before resume.
    pub(crate) fn remember_thread_history_mode(
        &mut self,
        thread_id: ThreadId,
        history_mode: ThreadHistoryMode,
    ) {
        if !self.uses_embedded_app_server() {
            return;
        }
        self.history_pagination
            .entry(thread_id)
            .or_default()
            .history_mode = history_mode;
    }

    pub(crate) async fn resume_thread(
        &mut self,
        config: Config,
        thread_id: ThreadId,
        model_settings: ResumeModelSettings,
    ) -> Result<AppServerStartedThread> {
        let session_config = if matches!(
            model_settings,
            ResumeModelSettings::RestoreFromThread | ResumeModelSettings::PreserveExistingThread
        ) {
            config.clone()
        } else {
            self.session_config_with_effective_service_tier(&config)
        };
        let mut params = thread_resume_params_from_config(
            session_config,
            thread_id,
            self.thread_params_mode(),
            self.remote_cwd_override.as_deref(),
            model_settings,
        );
        self.thread_tool_transport()
            .configure_mcp(&mut params.config);
        let mut rollout_maintenance_guard = None;
        params.exclude_turns = if self.history_support == ThreadHistorySupport::Paginated {
            let known_legacy_history = self
                .history_pagination
                .get(&thread_id)
                .is_some_and(|state| state.history_mode == ThreadHistoryMode::Legacy)
                && if !self.uses_embedded_app_server() {
                    true
                } else {
                    rollout_maintenance_guard =
                        codex_rollout::try_acquire_rollout_maintenance_lock(
                            config.codex_home.as_path(),
                        )
                        .ok()
                        .flatten();
                    rollout_maintenance_guard.is_some()
                        && self
                            .thread_read(thread_id, /*include_turns*/ false)
                            .await
                            .is_ok_and(|thread| thread.history_mode == ThreadHistoryMode::Legacy)
                };
            !known_legacy_history
        } else {
            false
        };
        if params.exclude_turns {
            rollout_maintenance_guard = None;
        }
        let request_id = self.next_request_id();
        let resume_response = self
            .client
            .request_typed(ClientRequest::ThreadResume {
                request_id,
                params: params.clone(),
            })
            .await;
        drop(rollout_maintenance_guard);
        let mut response: ThreadResumeResponse = match resume_response {
            Ok(response) => response,
            Err(TypedRequestError::Server { source, .. })
                if params.exclude_turns && is_history_pagination_unsupported(&source) =>
            {
                self.history_support = ThreadHistorySupport::LegacyOnly;
                params.exclude_turns = false;
                let request_id = self.next_request_id();
                self.client
                    .request_typed(ClientRequest::ThreadResume { request_id, params })
                    .await
                    .map_err(|err| {
                        bootstrap_request_error("thread/resume failed during TUI bootstrap", err)
                    })?
            }
            Err(TypedRequestError::Server { source, .. }) if is_active_writer_conflict(&source) => {
                let attached_response = if self.uses_embedded_app_server()
                    && let Some(client) =
                        crate::try_connect_default_local_app_server(config.codex_home.as_path())
                            .await
                {
                    tracing::info!(
                        thread_id = %thread_id,
                        "reattaching TUI to the default local app-server after writer conflict"
                    );
                    let original_client = std::mem::replace(&mut self.client, client);
                    let request_id = self.next_request_id();
                    match self
                        .client
                        .request_typed(ClientRequest::ThreadResume {
                            request_id,
                            params: params.clone(),
                        })
                        .await
                    {
                        Ok(response) => Some(response),
                        Err(_) => {
                            self.client = original_client;
                            None
                        }
                    }
                } else {
                    None
                };

                if let Some(response) = attached_response {
                    response
                } else {
                    let take_over =
                        crate::thread_takeover::offer(thread_id, config.codex_home.as_path())
                            .await
                            .wrap_err("failed to take over the active session")?;
                    if !take_over {
                        return Err(color_eyre::eyre::eyre!(
                            "thread/resume cancelled because the session is already open elsewhere"
                        ));
                    }
                    let request_id = self.next_request_id();
                    self.client
                        .request_typed(ClientRequest::ThreadResume { request_id, params })
                        .await
                        .map_err(|err| {
                            bootstrap_request_error(
                                "thread/resume failed during TUI bootstrap",
                                err,
                            )
                        })?
                }
            }
            Err(err) => {
                return Err(bootstrap_request_error(
                    "thread/resume failed during TUI bootstrap",
                    err,
                ));
            }
        };
        self.hydrate_initial_thread_history(
            &mut response.thread,
            response.turns_backwards_cursor.clone(),
            response.items_backwards_cursor.clone(),
            Some(&config),
            HistoryHydrationScope::Initial,
        )
        .await?;
        let fork_parent_title = self
            .fork_parent_title_from_app_server(response.thread.forked_from_id.as_deref())
            .await;
        let mut started =
            started_thread_from_resume_response(response, &config, self.thread_params_mode())
                .await?;
        started.session.fork_parent_title = fork_parent_title;
        if self.task_tools_available(thread_id) {
            self.remember_task_tool_thread(thread_id);
            started.task_tools_available = true;
        }
        Ok(started)
    }
}

#[cfg(test)]
#[path = "rollout_history_tests.rs"]
mod tests;
