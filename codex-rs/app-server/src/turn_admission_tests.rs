use super::TurnAdmission;
use crate::error_code::INVALID_REQUEST_ERROR_CODE;
use codex_extension_api::CoreWorkAdmission;
use codex_extension_api::CoreWorkKind;
use codex_extension_api::TurnStartAdmission;
use pretty_assertions::assert_eq;

#[test]
fn drain_rejects_new_work_without_waiting_for_admitted_work() {
    let admission = TurnAdmission::default();
    let active = admission.subscribe_active();
    let in_flight = admission.admit().expect("initial request admitted");
    let automatic = admission
        .admit_turn_start()
        .expect("automatic start admitted");
    admission.begin_drain();
    assert_eq!(*active.borrow(), 2);
    assert!(admission.admit_turn_start().is_none());

    let error = admission.admit().err().expect("new request rejected");
    assert_eq!(error.code, INVALID_REQUEST_ERROR_CODE);
    assert_eq!(
        error.data,
        Some(serde_json::json!({ "reason": "serverShuttingDown" }))
    );
    drop(in_flight);
    assert_eq!(*active.borrow(), 1);
    drop(automatic);
    assert_eq!(*active.borrow(), 0);
}

#[tokio::test]
async fn drain_rejects_new_model_requests_and_command_processes() {
    let admission = TurnAdmission::default();
    let active = admission.subscribe_active();
    let model_request = admission
        .acquire_work_start(CoreWorkKind::ModelRequest)
        .await
        .expect("initial model request admitted");
    let command_process = admission
        .acquire_work_start(CoreWorkKind::CommandProcessStart)
        .await
        .expect("initial command process admitted");

    admission.begin_drain();
    assert_eq!(*active.borrow(), 2);
    assert!(
        admission
            .acquire_work_start(CoreWorkKind::ModelRequest)
            .await
            .is_none()
    );
    assert!(
        admission
            .acquire_work_start(CoreWorkKind::CommandProcessStart)
            .await
            .is_none()
    );

    drop(model_request);
    drop(command_process);
    assert_eq!(*active.borrow(), 0);
}
