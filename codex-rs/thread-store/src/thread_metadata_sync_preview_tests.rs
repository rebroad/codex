//! Verify delegated inputs emit a preview-only live metadata patch exactly once.

use super::*;
use crate::ThreadPersistenceMetadata;
use codex_protocol::items::FunctionCallOutputItem;
use codex_protocol::models::FunctionCallOutputBody;
use codex_protocol::protocol::ItemCompletedEvent;
use codex_protocol::protocol::TurnStartedEvent;
use pretty_assertions::assert_eq;

#[tokio::test]
async fn ephemeral_create_skips_state_db_metadata_sync() {
    let thread_id = ThreadId::new();
    let mut sync = ThreadMetadataSync::for_create(&CreateThreadParams {
        creator_user_id: None,
        creator_account_id: None,
        session_id: thread_id.into(),
        thread_id,
        extra_config: None,
        forked_from_id: None,
        parent_thread_id: None,
        source: codex_protocol::protocol::SessionSource::Exec,
        thread_source: None,
        originator: String::new(),
        base_instructions: codex_protocol::models::BaseInstructions::default(),
        dynamic_tools: Vec::new(),
        selected_capability_roots: Vec::new(),
        multi_agent_version: None,
        history_mode: codex_protocol::protocol::ThreadHistoryMode::Legacy,
        history_base: None,
        subagent_history_start_ordinal: None,
        initial_window_id: uuid::Uuid::now_v7().to_string(),
        runtime_workspace_roots: None,
        metadata: ThreadPersistenceMetadata {
            ephemeral: true,
            cwd: None,
            model_provider: "test-provider".to_string(),
            memory_mode: ThreadMemoryMode::Enabled,
        },
    })
    .await;

    assert!(sync.take_pending_update().is_none());
    assert!(
        sync.observe_appended_items(&[RolloutItem::EventMsg(EventMsg::TurnStarted(
            TurnStartedEvent {
                turn_attribution: None,
                turn_id: "turn-1".to_string(),
                root_turn_id: None,
                trace_id: None,
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            },
        ))])
        .is_none()
    );
}

#[test]
fn delegated_output_emits_only_first_preview_in_live_patch() {
    let thread_id = ThreadId::new();
    let mut sync = ThreadMetadataSync::for_resume(
        &ResumeThreadParams {
            thread_id,
            rollout_path: None,
            history: None,
            history_revision: None,
            include_archived: false,
            metadata: ThreadPersistenceMetadata {
                ephemeral: false,
                cwd: None,
                model_provider: "test-provider".to_string(),
                memory_mode: ThreadMemoryMode::Enabled,
            },
        },
        /*metadata*/ None,
    );
    for (index, text) in ["first task", "later task"].into_iter().enumerate() {
        let item = RolloutItem::EventMsg(EventMsg::ItemCompleted(ItemCompletedEvent {
            thread_id,
            turn_id: "turn".to_string(),
            item: TurnItem::FunctionCallOutput(FunctionCallOutputItem {
                id: format!("output-{index}"),
                name: "create_thread".to_string(),
                namespace: Some("codex_tui".to_string()),
                output: FunctionCallOutputBody::Text(format!(
                    "<codex_delegation>\n  <source_thread_id>source</source_thread_id>\n  <input>{text}</input>\n</codex_delegation>"
                )),
            }),
            started_at_ms: Some(0),
            completed_at_ms: 0,
        }));
        let update = sync.observe_appended_items(&[item]);
        if index == 0 {
            let update = update.expect("first delegated preview patch");
            let expected = ThreadMetadataPatch {
                preview: Some(text.to_string()),
                updated_at: update.patch.updated_at,
                ..Default::default()
            };
            assert_eq!(
                serde_json::to_value(&update.patch).unwrap(),
                serde_json::to_value(expected).unwrap()
            );
            sync.mark_pending_update_applied(&update);
        } else {
            assert_eq!(update.and_then(|update| update.patch.preview), None);
        }
    }
}
