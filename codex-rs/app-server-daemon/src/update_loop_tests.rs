use pretty_assertions::assert_eq;

use super::install_latest_standalone;
use super::update_modes_for_identities;
use crate::RestartMode;
use crate::UpdaterRefreshMode;
use crate::managed_install::executable_identity_from_reader;

#[test]
fn unchanged_updater_uses_version_based_restart() -> std::io::Result<()> {
    assert_eq!(
        update_modes_for_identities(
            &executable_identity_from_reader(&b"same"[..])?,
            &executable_identity_from_reader(&b"same"[..])?,
        ),
        (RestartMode::IfVersionChanged, UpdaterRefreshMode::None)
    );
    Ok(())
}

#[test]
fn changed_updater_forces_refresh_even_when_version_may_match() -> std::io::Result<()> {
    assert_eq!(
        update_modes_for_identities(
            &executable_identity_from_reader(&b"old"[..])?,
            &executable_identity_from_reader(&b"new"[..])?,
        ),
        (
            RestartMode::Always,
            UpdaterRefreshMode::ReexecIfManagedBinaryChanged,
        )
    );
    Ok(())
}

#[tokio::test]
async fn standalone_installer_is_intentionally_a_noop() {
    // Termux packages update through the fork-owned release channel. The
    // upstream standalone installer must never fetch or execute here.
    install_latest_standalone()
        .await
        .expect("Termux standalone updater must fail closed as a no-op");
}
