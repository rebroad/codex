use std::path::Path;

use anyhow::Result;
use codex_protocol::protocol::TokenUsage;
use codex_state::AccountUsageStore;
use codex_state::usage_db_path;
use predicates::str::contains;
use sqlx::SqlitePool;
use tempfile::TempDir;

fn codex_command(codex_home: &Path) -> Result<assert_cmd::Command> {
    let mut cmd = assert_cmd::Command::new(codex_utils_cargo_bin::cargo_bin("codex")?);
    cmd.env("CODEX_HOME", codex_home);
    Ok(cmd)
}

#[test]
fn root_prompt_is_rejected_without_exec() -> Result<()> {
    let codex_home = TempDir::new()?;
    let mut cmd = codex_command(codex_home.path())?;
    cmd.arg("hello").assert().failure().stderr(contains(
        "Positional prompts are only supported via `codex exec`",
    ));
    Ok(())
}

#[tokio::test]
async fn usage_clear_all_accounts_deletes_default_provider_rows() -> Result<()> {
    let codex_home = TempDir::new()?;
    let usage_store =
        AccountUsageStore::init(codex_home.path().to_path_buf(), "openai".to_string()).await?;

    let usage = TokenUsage {
        total_tokens: 42,
        input_tokens: 30,
        cached_input_tokens: 0,
        output_tokens: 12,
        reasoning_output_tokens: 0,
    };
    usage_store
        .record_account_token_usage("account-1", &usage, None)
        .await?;

    let mut cmd = codex_command(codex_home.path())?;
    cmd.args(["usage", "clear", "--all-accounts", "--yes"])
        .assert()
        .success()
        .stdout(contains("Cleared usage tracking for all accounts"));

    let db_path = usage_db_path(codex_home.path());
    let pool = SqlitePool::connect(&format!("sqlite://{}", db_path.display())).await?;
    let usage_count: i64 = sqlx::query_scalar(
        r#"
SELECT COUNT(*) FROM account_usage
WHERE provider = ?
        "#,
    )
    .bind("openai")
    .fetch_one(&pool)
    .await?;
    assert_eq!(usage_count, 0);
    Ok(())
}
