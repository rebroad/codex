//! Covers retry-delay parsing for provider error messages.

use super::*;
use pretty_assertions::assert_eq;

#[test]
fn test_try_parse_retry_delay() {
    let err = Error {
        r#type: None,
        message: Some("Rate limit reached for gpt-5.1 in organization org- on tokens per min (TPM): Limit 1, Used 1, Requested 19304. Please try again in 28ms. Visit https://platform.openai.com/account/rate-limits to learn more.".to_string()),
        code: Some("rate_limit_exceeded".to_string()),
        plan_type: None,
        resets_at: None,
        limit_window_minutes: None,
        misalignment: None,
    };

    let delay = try_parse_retry_delay(&err);
    assert_eq!(delay, Some(Duration::from_millis(28)));
}

#[test]
fn test_try_parse_retry_delay_no_delay() {
    let err = Error {
        r#type: None,
        message: Some("Rate limit reached for gpt-5.1 in organization <ORG> on tokens per min (TPM): Limit 30000, Used 6899, Requested 24050. Please try again in 1.898s. Visit https://platform.openai.com/account/rate-limits to learn more.".to_string()),
        code: Some("rate_limit_exceeded".to_string()),
        plan_type: None,
        resets_at: None,
        limit_window_minutes: None,
        misalignment: None,
    };
    let delay = try_parse_retry_delay(&err);
    assert_eq!(delay, Some(Duration::from_secs_f64(1.898)));
}

#[test]
fn test_try_parse_retry_delay_azure() {
    let err = Error {
        r#type: None,
        message: Some("Rate limit exceeded. Try again in 35 seconds.".to_string()),
        code: Some("rate_limit_exceeded".to_string()),
        plan_type: None,
        resets_at: None,
        limit_window_minutes: None,
        misalignment: None,
    };
    let delay = try_parse_retry_delay(&err);
    assert_eq!(delay, Some(Duration::from_secs(35)));
}

#[test]
fn usage_limit_reached_is_classified_from_failed_response() {
    for error in [
        serde_json::json!({
            "type": "usage_limit_reached",
            "message": "You've hit your usage limit",
            "plan_type": "free",
            "resets_at": 1_795_803_019_i64,
            "limit_window_minutes": 43_200,
        }),
        serde_json::json!({
            "code": "usage_limit_reached",
            "message": "You've hit your usage limit",
        }),
    ] {
        let error = parse_failed_response(Some(serde_json::json!({
            "error": error,
        })));
        let ApiError::UsageLimitReached(error) = error else {
            panic!("expected usage-limit error, got {error:?}");
        };
        if error.plan_type.is_some() {
            assert_eq!(
                error.plan_type,
                Some(codex_protocol::auth::PlanType::from_raw_value("free"))
            );
            assert_eq!(error.limit_window_minutes, Some(43_200));
            assert_eq!(
                error.resets_at.map(|timestamp| timestamp.timestamp()),
                Some(1_795_803_019)
            );
        }
    }
}
