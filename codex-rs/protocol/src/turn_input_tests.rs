use super::PendingTurnInteraction;
use super::SuspendedInteractionModelContext;

#[test]
fn pending_interaction_checkpoint_round_trips_skipped_model_context() {
    let interaction = PendingTurnInteraction {
        correlation_id: "approval:call-1".to_string(),
        event: crate::protocol::EventMsg::ExecApprovalRequest(
            crate::approvals::ExecApprovalRequestEvent {
                model_context: Some(crate::items::ModelInvocationContext {
                    model_slug: "gpt-test".to_string(),
                    reasoning_effort: Some("high".to_string()),
                }),
                kind: crate::approvals::ExecApprovalKind::Command,
                call_id: "call-1".to_string(),
                plugin_id: None,
                script_path: None,
                approval_id: None,
                turn_id: "turn-1".to_string(),
                environment_id: None,
                started_at_ms: 1,
                command: vec!["echo".to_string()],
                cwd: codex_utils_path_uri::LegacyAppPathString::from_string("/tmp"),
                reason: None,
                network_approval_context: None,
                proposed_execpolicy_amendment: None,
                proposed_network_policy_amendments: None,
                additional_permissions: None,
                available_decisions: None,
                parsed_cmd: Vec::new(),
            },
        ),
        model_context: Some(SuspendedInteractionModelContext {
            model_slug: "gpt-test".to_string(),
            reasoning_effort: Some("high".to_string()),
        }),
    };

    let serialized = serde_json::to_value(&interaction).expect("serialize interaction checkpoint");
    let restored: PendingTurnInteraction =
        serde_json::from_value(serialized).expect("restore interaction checkpoint");

    assert_eq!(restored.correlation_id, interaction.correlation_id);
    assert_eq!(
        serde_json::to_value(&restored.event).expect("serialize restored event"),
        serde_json::to_value(&interaction.event).expect("serialize original event")
    );
    assert_eq!(restored.model_context, interaction.model_context);
}
