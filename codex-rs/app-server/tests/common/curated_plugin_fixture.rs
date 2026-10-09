use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

use anyhow::Context;
use anyhow::Result;
use serde_json::json;

/// Creates a local curated plugins Git remote and a global URL rewrite for test app-servers.
pub fn prepare_curated_plugin_git_fixture(codex_home: &Path) -> Result<PathBuf> {
    let source_root = codex_home.join(".tmp/test-curated-plugin-source");
    let repository = source_root.join("plugins.git");
    let marketplace_dir = repository.join(".agents/plugins");
    std::fs::create_dir_all(&marketplace_dir)?;
    for name in ["openai-curated", "openai-api-curated"] {
        let manifest = json!({ "name": name, "plugins": [] });
        std::fs::write(
            marketplace_dir.join(if name == "openai-curated" {
                "marketplace.json"
            } else {
                "api_marketplace.json"
            }),
            serde_json::to_vec_pretty(&manifest)?,
        )?;
    }

    if !repository.join(".git").is_dir() {
        run_git(&repository, &["init", "--quiet", "-b", "main"])?;
        run_git(
            &repository,
            &["config", "user.email", "codex-tests@openai.com"],
        )?;
        run_git(&repository, &["config", "user.name", "Codex Tests"])?;
        run_git(&repository, &["add", "."])?;
        run_git(
            &repository,
            &["commit", "--quiet", "-m", "curated plugins test fixture"],
        )?;
    }

    let global_config = codex_home.join(".tmp/test-curated-plugin-gitconfig");
    let remote_url = format!("file://{}/", source_root.display());
    let rewrite_key = format!("url.{remote_url}.insteadOf");
    let output = Command::new("git")
        .args(["config", "--file"])
        .arg(&global_config)
        .arg(rewrite_key)
        .arg("https://github.com/openai/")
        .output()
        .context("run git config for curated plugin test fixture")?;
    anyhow::ensure!(
        output.status.success(),
        "git config failed for curated plugin test fixture: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(global_config)
}

fn run_git(repository: &Path, args: &[&str]) -> Result<()> {
    let output = Command::new("git")
        .current_dir(repository)
        .args(args)
        .output()
        .context("run git for curated plugin test fixture")?;
    anyhow::ensure!(
        output.status.success(),
        "git {} failed for curated plugin test fixture: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}
