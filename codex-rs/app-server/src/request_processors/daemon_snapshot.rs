//! Checkpoints persistent turn trees before stopping them for daemon replacement.

use super::ThreadRequestProcessor;
use codex_app_server_transport::daemon_recovery::InterruptedTurn;
use codex_app_server_transport::daemon_recovery::RecoverySnapshot;
use codex_core::TurnTreeHandoff;
use codex_core::TurnTreeSuspendOutcome;
use codex_thread_store::PersistContext;
use std::io;
use std::path::PathBuf;
use tracing::warn;

impl ThreadRequestProcessor {
    /// Persist a complete recovery snapshot before stopping any active turn tree.
    ///
    /// Any preparation or persistence failure drops the prepared handoffs, reopening
    /// agent admission and leaving the old server in charge of its active work.
    #[expect(
        clippy::await_holding_invalid_type,
        reason = "handoff selection must be serialized against pending unloads"
    )]
    pub(crate) async fn daemon_handoff(&self, recovery_file: PathBuf) -> io::Result<()> {
        let mut snapshot = RecoverySnapshot::default();
        let mut handoffs: Vec<TurnTreeHandoff> = Vec::new();
        let thread_ids = self.thread_manager.list_thread_ids().await;

        for thread_id in thread_ids {
            let pending_unloads = self.pending_thread_unloads.lock().await;
            if pending_unloads.contains_key(&thread_id) {
                continue;
            }
            let Ok(thread) = self.thread_manager.get_thread(thread_id).await else {
                continue;
            };
            let config = thread.config_snapshot().await;
            if config.ephemeral
                || config.parent_thread_id.is_some()
                || config.session_source.is_non_root_agent()
            {
                continue;
            }

            // The root owns the runtime seal for its loaded descendants. Prepare
            // before persisting so accepted mailbox input and pending processes are
            // part of the same generation's recovery record.
            let handoff = if thread.interrupted_turn().await.is_some() {
                Some(thread.prepare_turn_tree_handoff().await.map_err(|err| {
                    io::Error::other(format!("prepare thread {thread_id}: {err}"))
                })?)
            } else {
                None
            };

            self.thread_store
                .persist_thread(thread_id, PersistContext::Standard)
                .await
                .map_err(|err| io::Error::other(format!("persist thread {thread_id}: {err}")))?;
            snapshot.loaded.insert(thread_id.to_string());

            if let Some(handoff) = handoff {
                for checkpoint in handoff.checkpoints() {
                    let saved_thread = self
                        .thread_manager
                        .get_thread(checkpoint.thread_id)
                        .await
                        .map_err(|err| {
                        io::Error::other(format!(
                            "load thread {} for handoff: {err}",
                            checkpoint.thread_id
                        ))
                    })?;
                    let config = saved_thread.config_snapshot().await;
                    let [environment] = config.environment_selections() else {
                        return Err(io::Error::other(format!(
                            "thread {} has an ambiguous executor environment",
                            checkpoint.thread_id
                        )));
                    };
                    let interrupted = saved_thread.interrupted_turn().await;
                    let options = interrupted
                        .as_ref()
                        .map(|(_, options, _)| options.clone())
                        .unwrap_or_default();
                    snapshot.loaded.insert(checkpoint.thread_id.to_string());
                    snapshot.interrupted.insert(
                        checkpoint.thread_id.to_string(),
                        InterruptedTurn {
                            turn_id: checkpoint.turn_id.clone(),
                            output_schema: options.final_output_json_schema,
                            service_tier: options.service_tier,
                            cyber_access_program: options.cyber_access_program,
                            local_environment: Some((&(*environment)).into()),
                            suspended: Some(checkpoint.clone()),
                        },
                    );
                }
                // Persist descendants as independently resumable threads too.
                for checkpoint in handoff.checkpoints() {
                    self.thread_store
                        .persist_thread(checkpoint.thread_id, PersistContext::Standard)
                        .await
                        .map_err(|err| {
                            io::Error::other(format!(
                                "persist thread {}: {err}",
                                checkpoint.thread_id
                            ))
                        })?;
                }
                handoffs.push(handoff);
            }
        }

        // Startup recovery may still be waiting for a client to replay an
        // interaction. Carry those not-yet-started turns into this generation's
        // handoff instead of replacing their only durable checkpoint.
        let prior_path = recovery_file.clone();
        if let Ok(Ok(prior)) = tokio::task::spawn_blocking(move || {
            codex_app_server_transport::daemon_recovery::read_snapshot(&prior_path)
        })
        .await
        {
            snapshot.loaded.extend(prior.loaded);
            for (thread_id, interrupted) in prior.interrupted {
                snapshot.interrupted.entry(thread_id).or_insert(interrupted);
            }
        }

        // The atomic file is the ownership transfer point: no old turn is stopped
        // before the replacement has enough information to recover it.
        crate::daemon_thread_recovery::snapshot(recovery_file, snapshot).await?;

        for mut handoff in handoffs {
            match handoff.commit().await {
                Ok(
                    TurnTreeSuspendOutcome::Suspended { .. } | TurnTreeSuspendOutcome::NotActive,
                ) => {}
                Ok(TurnTreeSuspendOutcome::PartialCommit {
                    failed_thread_id,
                    error,
                    ..
                }) => {
                    warn!(%failed_thread_id, %error, "turn tree only partially suspended; replacement will recover from the durable checkpoint");
                }
                Ok(outcome) => {
                    warn!(
                        ?outcome,
                        "turn tree could not be fully suspended after its checkpoint was persisted"
                    );
                }
                Err(err) => {
                    warn!(%err, "turn tree suspension failed after its checkpoint was persisted");
                }
            }
        }
        Ok(())
    }
}
