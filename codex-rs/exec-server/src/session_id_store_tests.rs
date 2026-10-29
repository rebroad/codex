use pretty_assertions::assert_eq;

use super::load;
use super::resume_session_id;
use super::save;

#[tokio::test]
async fn session_id_store_round_trips_and_replaces_the_saved_id() -> anyhow::Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let path = temp_dir.path().join("session-id");

    assert_eq!(load(&path).await?, None);
    save(&path, "session-1").await?;
    assert_eq!(load(&path).await?.as_deref(), Some("session-1"));
    save(&path, "session-2").await?;
    assert_eq!(load(&path).await?.as_deref(), Some("session-2"));
    Ok(())
}

#[tokio::test]
async fn resume_session_id_prefers_explicit_id_then_uses_the_saved_id() -> anyhow::Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let path = temp_dir.path().join("session-id");
    save(&path, "saved-session").await?;

    assert_eq!(
        resume_session_id(None, Some(&path)).await?.as_deref(),
        Some("saved-session")
    );
    assert_eq!(
        resume_session_id(Some("explicit-session".to_string()), Some(&path))
            .await?
            .as_deref(),
        Some("explicit-session")
    );
    assert_eq!(resume_session_id(None, None).await?, None);
    Ok(())
}

#[tokio::test]
async fn empty_session_id_file_is_rejected() -> anyhow::Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let path = temp_dir.path().join("session-id");
    tokio::fs::write(&path, "  \n").await?;

    assert!(load(&path).await.is_err());
    Ok(())
}
