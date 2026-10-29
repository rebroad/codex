//! Turn-scoped state, input waiters, and active turn metadata.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;
use tokio_util::task::AbortOnDropHandle;

use codex_diagnostics::GaugeGuard;
use codex_protocol::dynamic_tools::DynamicToolResponse;
use codex_protocol::request_permissions::RequestPermissionProfile;
use codex_protocol::request_permissions::RequestPermissionsResponse;
use codex_protocol::request_user_input::RequestUserInputResponse;
use codex_rmcp_client::ElicitationResponse;
use rmcp::model::RequestId;
use tokio::sync::oneshot;

use super::TurnTokenUsage;
use crate::agent::types::AgentExecutionGuard;
use crate::session::TurnInputQueue;
use crate::session::step_context::StepContext;
use crate::session::turn_context::TurnContext;
use crate::session::turn_context::TurnEnvironment;
use crate::tasks::AnySessionTask;
use codex_protocol::protocol::ReviewDecision;
use codex_protocol::protocol::TokenUsage;
use codex_protocol::turn_input::PendingTurnInteraction;

/// Metadata about the currently running turn.
pub(crate) struct ActiveTurn {
    pub(crate) task: Option<RunningTask>,
    /// Present when input admission is closed; cancelled when the final batch is recorded.
    /// Reservations that have not started still accept inputs.
    pub(crate) finishing: Option<CancellationToken>,
    pub(crate) turn_state: Arc<Mutex<TurnState>>,
}

/// Whether mailbox deliveries should still be folded into the current turn.
///
/// State machine:
/// - A turn starts in `CurrentTurn`, so queued child mail can join the next
///   model request for that turn.
/// - After user-visible terminal output is recorded, we switch to `NextTurn`
///   to leave late child mail queued instead of extending an already shown
///   answer.
/// - If the same task later gets explicit same-turn work again (a steered user
///   prompt or a tool call after an untagged preamble), we reopen `CurrentTurn`
///   so that pending child mail is drained into that follow-up request.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum MailboxDeliveryPhase {
    /// Incoming mailbox messages can still be consumed by the current turn.
    #[default]
    CurrentTurn,
    /// The current turn already emitted visible final answer text; mailbox
    /// messages should remain queued for a later turn.
    NextTurn,
}

impl Default for ActiveTurn {
    fn default() -> Self {
        Self {
            task: None,
            finishing: None,
            turn_state: Arc::new(Mutex::new(TurnState::default())),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TaskKind {
    Regular,
    Review,
    Compact,
}

pub(crate) struct RunningTask {
    pub(crate) done: Arc<Notify>,
    pub(crate) kind: TaskKind,
    pub(crate) task: Arc<dyn AnySessionTask>,
    pub(crate) cancellation_token: CancellationToken,
    pub(crate) handle: AbortOnDropHandle<()>,
    pub(crate) turn_context: Arc<TurnContext>,
    pub(crate) _agent_execution_guard: Option<AgentExecutionGuard>,
    pub(crate) _diagnostics_guard: GaugeGuard,
    // Timer recorded when the task drops to capture the full turn duration.
    pub(crate) _timer: Option<codex_otel::Timer>,
}

/// Mutable state for a single turn.
#[derive(Default)]
pub(crate) struct TurnState {
    pending_approvals: HashMap<String, oneshot::Sender<ReviewDecision>>,
    pending_request_permissions: HashMap<String, PendingRequestPermissions>,
    pending_user_input: HashMap<String, oneshot::Sender<AcceptedUserInputResponse>>,
    pending_elicitations: HashMap<(String, RequestId), oneshot::Sender<ElicitationResponse>>,
    pending_dynamic_tools: HashMap<String, oneshot::Sender<DynamicToolResponse>>,
    pending_interactions: HashMap<String, PendingTurnInteraction>,
    recovered_interactions: HashMap<String, PendingTurnInteraction>,
    pub(crate) pending_input: TurnInputQueue,
    mailbox_delivery_phase: MailboxDeliveryPhase,
    pub(crate) tool_calls: u64,
    pub(crate) has_memory_citation: bool,
    pub(crate) token_usage_at_turn_start: TokenUsage,
    pub(crate) token_usage_by_model: TurnTokenUsage,
    /// The last step captured for execution or selected from a speculative fallback.
    /// Remains absent until a step is captured; standalone local compaction has no step.
    pub(crate) last_known_step_context: Option<Arc<StepContext>>,
}

/// Host receipt metadata follows the response through the asynchronous tool waiter.
pub(crate) struct AcceptedUserInputResponse {
    pub(crate) response: RequestUserInputResponse,
    /// Order reserved when the host accepts this response.
    pub(crate) acceptance_order: u64,
}

pub(crate) struct PendingRequestPermissions {
    pub(crate) tx_response: oneshot::Sender<RequestPermissionsResponse>,
    pub(crate) requested_permissions: RequestPermissionProfile,
    pub(crate) environment: TurnEnvironment,
    /// Background cells can request permissions after a different turn becomes active.
    pub(crate) turn_context: Arc<TurnContext>,
}

impl TurnState {
    pub(crate) fn insert_pending_approval(
        &mut self,
        key: String,
        tx: oneshot::Sender<ReviewDecision>,
    ) -> Option<oneshot::Sender<ReviewDecision>> {
        self.pending_approvals.insert(key, tx)
    }

    pub(crate) fn remove_pending_approval(
        &mut self,
        key: &str,
    ) -> Option<oneshot::Sender<ReviewDecision>> {
        self.pending_interactions
            .remove(&interaction_key("approval", key));
        self.pending_approvals.remove(key)
    }

    pub(crate) fn clear_pending_waiters(&mut self) {
        self.pending_approvals.clear();
        self.pending_request_permissions.clear();
        self.pending_user_input.clear();
        self.pending_elicitations.clear();
        self.pending_dynamic_tools.clear();
        self.pending_interactions.clear();
        self.recovered_interactions.clear();
    }

    pub(crate) fn insert_pending_request_permissions(
        &mut self,
        key: String,
        pending_request_permissions: PendingRequestPermissions,
    ) -> Option<PendingRequestPermissions> {
        self.pending_request_permissions
            .insert(key, pending_request_permissions)
    }

    pub(crate) fn remove_pending_request_permissions(
        &mut self,
        key: &str,
    ) -> Option<PendingRequestPermissions> {
        self.pending_interactions
            .remove(&interaction_key("permissions", key));
        self.pending_request_permissions.remove(key)
    }

    pub(crate) fn insert_pending_user_input(
        &mut self,
        key: String,
        tx: oneshot::Sender<AcceptedUserInputResponse>,
    ) -> Option<oneshot::Sender<AcceptedUserInputResponse>> {
        self.pending_user_input.insert(key, tx)
    }

    pub(crate) fn remove_pending_user_input(
        &mut self,
        key: &str,
    ) -> Option<oneshot::Sender<AcceptedUserInputResponse>> {
        self.pending_interactions
            .remove(&interaction_key("user_input", key));
        self.pending_user_input.remove(key)
    }

    pub(crate) fn insert_pending_elicitation(
        &mut self,
        server_name: String,
        request_id: RequestId,
        tx: oneshot::Sender<ElicitationResponse>,
    ) -> Option<oneshot::Sender<ElicitationResponse>> {
        self.pending_elicitations
            .insert((server_name, request_id), tx)
    }

    pub(crate) fn remove_pending_elicitation(
        &mut self,
        server_name: &str,
        request_id: &RequestId,
    ) -> Option<oneshot::Sender<ElicitationResponse>> {
        let key = format!("{server_name}:{}", request_id);
        self.pending_interactions
            .remove(&interaction_key("elicitation", &key));
        self.pending_elicitations
            .remove(&(server_name.to_string(), request_id.clone()))
    }

    pub(crate) fn insert_pending_dynamic_tool(
        &mut self,
        key: String,
        tx: oneshot::Sender<DynamicToolResponse>,
    ) -> Option<oneshot::Sender<DynamicToolResponse>> {
        self.pending_dynamic_tools.insert(key, tx)
    }

    pub(crate) fn remove_pending_dynamic_tool(
        &mut self,
        key: &str,
    ) -> Option<oneshot::Sender<DynamicToolResponse>> {
        self.pending_interactions
            .remove(&interaction_key("dynamic_tool", key));
        self.pending_dynamic_tools.remove(key)
    }

    pub(crate) fn record_pending_interaction(
        &mut self,
        kind: &str,
        key: &str,
        event: codex_protocol::protocol::EventMsg,
    ) {
        let correlation_id = interaction_key(kind, key);
        let replay_key = interaction_replay_key(&event);
        if let Some(recovered_id) = self
            .recovered_interactions
            .iter()
            .find_map(|(id, recovered)| {
                (interaction_replay_key(&recovered.event) == replay_key).then(|| id.clone())
            })
            .or_else(|| {
                let mut matching = self
                    .recovered_interactions
                    .iter()
                    .filter(|(_, recovered)| same_elicitation_server(&recovered.event, &event));
                let (id, _) = matching.next()?;
                matching.next().is_none().then(|| id.clone())
            })
        {
            self.recovered_interactions.remove(&recovered_id);
        }
        self.pending_interactions.insert(
            correlation_id.clone(),
            PendingTurnInteraction {
                correlation_id,
                model_context: match &event {
                    codex_protocol::protocol::EventMsg::ExecApprovalRequest(event) => {
                        event.model_context.as_ref().map(|context| {
                            codex_protocol::turn_input::SuspendedInteractionModelContext {
                                model_slug: context.model_slug.clone(),
                                reasoning_effort: context.reasoning_effort.clone(),
                            }
                        })
                    }
                    _ => None,
                },
                event,
            },
        );
    }

    pub(crate) fn pending_interactions(&self) -> Vec<PendingTurnInteraction> {
        let mut interactions = self
            .pending_interactions
            .values()
            .cloned()
            .collect::<Vec<_>>();
        interactions.extend(self.recovered_interactions.values().cloned());
        interactions.sort_by(|left, right| left.correlation_id.cmp(&right.correlation_id));
        interactions
    }

    pub(crate) fn restore_pending_interactions(
        &mut self,
        interactions: Vec<PendingTurnInteraction>,
    ) {
        self.recovered_interactions.extend(
            interactions
                .into_iter()
                .map(|interaction| (interaction.correlation_id.clone(), interaction)),
        );
    }

    pub(crate) fn accept_mailbox_delivery_for_current_turn(&mut self) {
        self.set_mailbox_delivery_phase(MailboxDeliveryPhase::CurrentTurn);
    }

    pub(crate) fn accepts_mailbox_delivery_for_current_turn(&self) -> bool {
        self.mailbox_delivery_phase == MailboxDeliveryPhase::CurrentTurn
    }

    pub(crate) fn set_mailbox_delivery_phase(&mut self, phase: MailboxDeliveryPhase) {
        self.mailbox_delivery_phase = phase;
    }
}

/// Produces a semantic key that ignores IDs and timestamps regenerated while a turn resumes.
fn interaction_replay_key(event: &codex_protocol::protocol::EventMsg) -> String {
    let mut value = serde_json::to_value(event).unwrap_or_default();
    let volatile_fields: &[&str] = match event {
        codex_protocol::protocol::EventMsg::ExecApprovalRequest(_)
        | codex_protocol::protocol::EventMsg::ApplyPatchApprovalRequest(_)
        | codex_protocol::protocol::EventMsg::RequestPermissions(_)
        | codex_protocol::protocol::EventMsg::RequestUserInput(_)
        | codex_protocol::protocol::EventMsg::DynamicToolCallRequest(_) => {
            &["call_id", "approval_id", "turn_id", "started_at_ms"]
        }
        codex_protocol::protocol::EventMsg::ElicitationRequest(_) => &["id", "turn_id"],
        _ => &[],
    };
    if let Some(object) = value.as_object_mut() {
        for field in volatile_fields {
            object.remove(*field);
        }
    }
    serde_json::to_string(&value).unwrap_or_default()
}

fn same_elicitation_server(
    left: &codex_protocol::protocol::EventMsg,
    right: &codex_protocol::protocol::EventMsg,
) -> bool {
    use codex_protocol::protocol::EventMsg;
    match (left, right) {
        (EventMsg::ElicitationRequest(left), EventMsg::ElicitationRequest(right)) => {
            left.server_name == right.server_name
        }
        _ => false,
    }
}

fn interaction_key(kind: &str, key: &str) -> String {
    format!("{kind}:{key}")
}
