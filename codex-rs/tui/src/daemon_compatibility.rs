//! Non-blocking compatibility reporting for an already-running app-server.

use crate::AppServerTarget;
use crate::daemon_startup;
use crate::legacy_core::config::Config;
use crate::startup_draft::StartupDraft;
use std::io;

pub(super) async fn check(
    startup: &mut StartupDraft,
    target: &AppServerTarget,
    config: &Config,
) -> io::Result<Option<String>> {
    let result = startup
        .run_until(daemon_startup::compatibility_warning(target, config))
        .await?;
    Ok(compatibility_note(result))
}

fn compatibility_note(result: Result<Option<String>, String>) -> Option<String> {
    match result {
        Ok(warning) => warning,
        Err(issue) => Some(format!(
            "Continuing with the existing background server: {issue}."
        )),
    }
}

#[cfg(test)]
#[path = "daemon_compatibility_tests.rs"]
mod tests;
