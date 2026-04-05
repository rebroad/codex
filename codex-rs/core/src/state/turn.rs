//! Turn-scoped state and active turn metadata scaffolding.

use codex_sandboxing::policy_transforms::merge_permission_profiles;
use indexmap::IndexMap;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;
use tokio_util::task::AbortOnDropHandle;

use codex_protocol::dynamic_tools::DynamicToolResponse;
use codex_protocol::models::ResponseInputItem;
use codex_protocol::request_permissions::RequestPermissionsResponse;
use codex_protocol::request_user_input::RequestUserInputResponse;
use codex_rmcp_client::ElicitationResponse;
use rmcp::model::RequestId;
use tokio::sync::oneshot;

use crate::codex::TurnContext;
use crate::protocol::ReviewDecision;
use crate::protocol::TokenUsage;
use crate::tasks::AnySessionTask;
use codex_protocol::models::PermissionProfile;

/// Metadata about the currently running turn.
pub(crate) struct ActiveTurn {
    pub(crate) tasks: IndexMap<String, RunningTask>,
    pub(crate) turn_state: Arc<Mutex<TurnState>>,
}

impl Default for ActiveTurn {
    fn default() -> Self {
        Self {
            tasks: IndexMap::new(),
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
    pub(crate) handle: Arc<AbortOnDropHandle<()>>,
    pub(crate) turn_context: Arc<TurnContext>,
    // Timer recorded when the task drops to capture the full turn duration.
    pub(crate) _timer: Option<codex_otel::Timer>,
}

impl ActiveTurn {
    pub(crate) fn add_task(&mut self, task: RunningTask) {
        let sub_id = task.turn_context.sub_id.clone();
        self.tasks.insert(sub_id, task);
    }

    pub(crate) fn remove_task(&mut self, sub_id: &str) -> bool {
        self.tasks.swap_remove(sub_id);
        self.tasks.is_empty()
    }

    pub(crate) fn drain_tasks(&mut self) -> Vec<RunningTask> {
        self.tasks.drain(..).map(|(_, task)| task).collect()
    }
}

/// Mutable state for a single turn.
#[derive(Default)]
pub(crate) struct TurnState {
    pending_approvals: HashMap<String, oneshot::Sender<ReviewDecision>>,
    pending_request_permissions: HashMap<String, oneshot::Sender<RequestPermissionsResponse>>,
    pending_user_input: HashMap<String, oneshot::Sender<RequestUserInputResponse>>,
    pending_elicitations: HashMap<(String, RequestId), oneshot::Sender<ElicitationResponse>>,
    pending_dynamic_tools: HashMap<String, oneshot::Sender<DynamicToolResponse>>,
    pending_input: Vec<ResponseInputItem>,
    granted_permissions: Option<PermissionProfile>,
    tool_calls_blocked_pending_steer: bool,
    blocked_tool_guidance_emitted: bool,
    sampling_request_cancellation_token: Option<CancellationToken>,
    restart_sampling_after_steer: bool,
    pub(crate) tool_calls: u64,
    pub(crate) token_usage_at_turn_start: TokenUsage,
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
        self.pending_approvals.remove(key)
    }

    pub(crate) fn clear_pending(&mut self) {
        self.pending_approvals.clear();
        self.pending_request_permissions.clear();
        self.pending_user_input.clear();
        self.pending_elicitations.clear();
        self.pending_dynamic_tools.clear();
        self.pending_input.clear();
        self.tool_calls_blocked_pending_steer = false;
        self.blocked_tool_guidance_emitted = false;
        self.sampling_request_cancellation_token = None;
        self.restart_sampling_after_steer = false;
    }

    pub(crate) fn insert_pending_request_permissions(
        &mut self,
        key: String,
        tx: oneshot::Sender<RequestPermissionsResponse>,
    ) -> Option<oneshot::Sender<RequestPermissionsResponse>> {
        self.pending_request_permissions.insert(key, tx)
    }

    pub(crate) fn remove_pending_request_permissions(
        &mut self,
        key: &str,
    ) -> Option<oneshot::Sender<RequestPermissionsResponse>> {
        self.pending_request_permissions.remove(key)
    }

    pub(crate) fn insert_pending_user_input(
        &mut self,
        key: String,
        tx: oneshot::Sender<RequestUserInputResponse>,
    ) -> Option<oneshot::Sender<RequestUserInputResponse>> {
        self.pending_user_input.insert(key, tx)
    }

    pub(crate) fn remove_pending_user_input(
        &mut self,
        key: &str,
    ) -> Option<oneshot::Sender<RequestUserInputResponse>> {
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
        self.pending_dynamic_tools.remove(key)
    }

    pub(crate) fn push_pending_input(&mut self, input: ResponseInputItem) {
        self.pending_input.push(input);
    }

    pub(crate) fn prepend_pending_input(&mut self, mut input: Vec<ResponseInputItem>) {
        if input.is_empty() {
            return;
        }

        input.append(&mut self.pending_input);
        self.pending_input = input;
    }

    pub(crate) fn take_pending_input(&mut self) -> Vec<ResponseInputItem> {
        if self.pending_input.is_empty() {
            Vec::with_capacity(0)
        } else {
            let mut ret = Vec::new();
            std::mem::swap(&mut ret, &mut self.pending_input);
            ret
        }
    }

    pub(crate) fn has_pending_input(&self) -> bool {
        !self.pending_input.is_empty()
    }

    pub(crate) fn block_tool_calls_pending_steer(&mut self) {
        self.tool_calls_blocked_pending_steer = true;
    }

    pub(crate) fn unblock_tool_calls_after_steer(&mut self) {
        self.tool_calls_blocked_pending_steer = false;
        self.blocked_tool_guidance_emitted = false;
    }

    pub(crate) fn tool_calls_blocked_pending_steer(&self) -> bool {
        self.tool_calls_blocked_pending_steer
    }

    pub(crate) fn increment_tool_calls(&mut self) {
        self.tool_calls = self.tool_calls.saturating_add(1);
    }

    pub(crate) fn mark_blocked_tool_guidance_emitted_if_first(&mut self) -> bool {
        if self.blocked_tool_guidance_emitted {
            false
        } else {
            self.blocked_tool_guidance_emitted = true;
            true
        }
    }

    pub(crate) fn set_sampling_request_cancellation_token(&mut self, token: CancellationToken) {
        self.sampling_request_cancellation_token = Some(token);
    }

    pub(crate) fn clear_sampling_request_cancellation_token(&mut self) {
        self.sampling_request_cancellation_token = None;
    }

    pub(crate) fn cancel_active_sampling_request(&mut self) -> bool {
        if let Some(token) = self.sampling_request_cancellation_token.take() {
            token.cancel();
            true
        } else {
            false
        }
    }

    pub(crate) fn request_sampling_restart_after_steer(&mut self) {
        self.restart_sampling_after_steer = true;
    }

    pub(crate) fn take_sampling_restart_after_steer(&mut self) -> bool {
        std::mem::take(&mut self.restart_sampling_after_steer)
    }

    pub(crate) fn record_granted_permissions(&mut self, permissions: PermissionProfile) {
        self.granted_permissions =
            merge_permission_profiles(self.granted_permissions.as_ref(), Some(&permissions));
    }

    pub(crate) fn granted_permissions(&self) -> Option<PermissionProfile> {
        self.granted_permissions.clone()
    }
}

impl ActiveTurn {
    /// Clear any pending approvals and input buffered for the current turn.
    pub(crate) async fn clear_pending(&self) {
        let mut ts = self.turn_state.lock().await;
        ts.clear_pending();
    }
}

#[cfg(test)]
mod tests {
    use super::TurnState;
    use tokio_util::sync::CancellationToken;

    #[test]
    fn tool_call_blocking_flag_defaults_to_unblocked() {
        let turn_state = TurnState::default();
        assert!(!turn_state.tool_calls_blocked_pending_steer());
    }

    #[test]
    fn tool_call_blocking_flag_can_be_blocked_and_unblocked() {
        let mut turn_state = TurnState::default();
        turn_state.block_tool_calls_pending_steer();
        assert!(turn_state.tool_calls_blocked_pending_steer());

        turn_state.unblock_tool_calls_after_steer();
        assert!(!turn_state.tool_calls_blocked_pending_steer());
    }

    #[test]
    fn clearing_pending_state_unblocks_tool_calls() {
        let mut turn_state = TurnState::default();
        turn_state.block_tool_calls_pending_steer();
        assert!(turn_state.tool_calls_blocked_pending_steer());

        turn_state.clear_pending();
        assert!(!turn_state.tool_calls_blocked_pending_steer());
    }

    #[test]
    fn cancel_active_sampling_request_is_one_shot() {
        let mut turn_state = TurnState::default();
        let token = CancellationToken::new();
        turn_state.set_sampling_request_cancellation_token(token.clone());

        assert!(turn_state.cancel_active_sampling_request());
        assert!(token.is_cancelled());
        assert!(!turn_state.cancel_active_sampling_request());
    }

    #[test]
    fn sampling_restart_flag_is_one_shot() {
        let mut turn_state = TurnState::default();
        assert!(!turn_state.take_sampling_restart_after_steer());

        turn_state.request_sampling_restart_after_steer();
        assert!(turn_state.take_sampling_restart_after_steer());
        assert!(!turn_state.take_sampling_restart_after_steer());
    }
}
