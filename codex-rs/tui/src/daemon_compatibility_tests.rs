use super::*;
use pretty_assertions::assert_eq;

#[test]
fn feature_mismatch_keeps_the_existing_background_server() {
    let note = compatibility_note(Err(daemon_startup::CompatibilityError {
        reason: "This session requires api_key_model_discovery to be enabled".to_string(),
        restart_features: None,
    }));

    assert_eq!(
        note.as_deref(),
        Some(
            "Continuing with the existing background server: This session requires api_key_model_discovery to be enabled."
        )
    );
}

#[test]
fn compatible_background_server_has_no_warning() {
    assert_eq!(compatibility_note(Ok(None)), None);
}
