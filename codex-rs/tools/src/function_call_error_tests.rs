use super::FunctionCallError;

#[test]
fn malformed_arguments_preserve_model_message_and_have_failure_kind() {
    let error = FunctionCallError::MalformedArguments("unknown field `value`".to_string());

    assert_eq!(
        error.to_string(),
        "failed to parse function arguments: unknown field `value`"
    );
    assert_eq!(error.failure_kind(), Some("parse_arguments"));
}

#[test]
fn other_failures_do_not_have_parse_failure_kind() {
    assert_eq!(
        FunctionCallError::RespondToModel(
            "failed to parse function arguments: unknown field `value`".to_string()
        )
        .failure_kind(),
        Some("parse_arguments")
    );
    assert_eq!(
        FunctionCallError::RespondToModel("denied".to_string()).failure_kind(),
        None
    );
    assert_eq!(
        FunctionCallError::Fatal("broken".to_string()).failure_kind(),
        None
    );
}
