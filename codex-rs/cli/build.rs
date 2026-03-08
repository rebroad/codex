use std::path::Path;
use std::process::Command;

fn run_git(repo_dir: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .current_dir(repo_dir)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn main() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string());
    let repo_dir = Path::new(&manifest_dir);

    let upstream_version = run_git(
        repo_dir,
        &[
            "describe",
            "--tags",
            "--match",
            "rust-v*",
            "--abbrev=0",
            "HEAD",
        ],
    )
    .or_else(|| {
        run_git(
            repo_dir,
            &["describe", "--tags", "--match", "rust-v*", "--abbrev=0"],
        )
    })
    .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string());

    let short_sha =
        run_git(repo_dir, &["rev-parse", "--short=9", "HEAD"]).unwrap_or_else(|| "unknown".into());

    let version = format!("{upstream_version}+{short_sha}");
    println!("cargo:rustc-env=CODEX_CLI_VERSION={version}");
    println!("cargo:rerun-if-changed=../.git/HEAD");
    println!("cargo:rerun-if-env-changed=CODEX_CLI_VERSION");
}
