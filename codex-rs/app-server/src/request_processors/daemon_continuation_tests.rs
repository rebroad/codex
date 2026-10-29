use super::wait_for_pending_interaction_client;
use crate::outgoing_message::ConnectionId;
use crate::thread_state::ConnectionCapabilities;
use crate::thread_state::ThreadStateManager;
use codex_protocol::ThreadId;
use std::time::Duration;

#[tokio::test]
async fn recovered_interaction_waits_for_a_thread_client() -> anyhow::Result<()> {
    let manager = ThreadStateManager::new();
    let thread_id = ThreadId::from_string("ba62fd70-2ec2-4b1b-9d94-355694332dd2")?;
    let connection_id = ConnectionId(1);
    manager
        .connection_initialized(connection_id, ConnectionCapabilities::default())
        .await;

    let waiting = wait_for_pending_interaction_client(&manager, thread_id);
    tokio::pin!(waiting);
    assert!(
        tokio::time::timeout(Duration::from_millis(25), &mut waiting)
            .await
            .is_err()
    );

    assert!(
        manager
            .try_add_connection_to_thread(thread_id, connection_id)
            .await
    );
    tokio::time::timeout(Duration::from_secs(1), &mut waiting).await?;
    Ok(())
}
