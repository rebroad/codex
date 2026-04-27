use std::path::Path;

use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::RolloutItem;
use codex_protocol::protocol::RolloutLine;
use codex_protocol::protocol::TokenUsageInfo;
#[derive(Debug, Clone, Default)]
pub struct RolloutThreadSnapshot {
    pub latest_turn_context: Option<codex_protocol::protocol::TurnContextItem>,
    pub latest_token_usage_info: Option<TokenUsageInfo>,
}

pub async fn read_rollout_thread_snapshot(path: &Path) -> Option<RolloutThreadSnapshot> {
    let text = tokio::fs::read_to_string(path).await.ok()?;
    let mut snapshot = RolloutThreadSnapshot::default();
    for line in text.lines().rev() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(rollout_line) = serde_json::from_str::<RolloutLine>(trimmed) else {
            continue;
        };
        match rollout_line.item {
            RolloutItem::TurnContext(item) if snapshot.latest_turn_context.is_none() => {
                snapshot.latest_turn_context = Some(item);
            }
            RolloutItem::EventMsg(EventMsg::TokenCount(event))
                if snapshot.latest_token_usage_info.is_none() =>
            {
                snapshot.latest_token_usage_info = event.info;
            }
            _ => {}
        }
        if snapshot.latest_turn_context.is_some() && snapshot.latest_token_usage_info.is_some() {
            break;
        }
    }
    Some(snapshot)
}
