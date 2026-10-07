# Alpha history: skipped commit audit

## Rearrangement audit (2026-10-07)

The clean starting `alpha` commit was `efbf5f7d83ea7d8b25c8d6be98dd2f30691da640`,
tree `9589f0af3a3fc1528a7ebf77a20b375c520c3242`, with merge-base
`635a0c0b1353e22c8783ef15652c3985078cb2c5` (`Release 0.162.0-alpha.9`) and 125
downstream commits. The uniquely named checkpoint
`alpha.before-rearrange-20261007-goal-split` points to that exact commit and
tree. Keep the merge-base fixed and require the final tree to equal this tree.

The downstream range contained 14 `fixup!` commits. Three have direct
downstream owners: takeover fixup `157abfa155` belongs to `308af1a2e0` (active
session takeover); bare-prompt fixup `8064eaab05` belongs to `715c671132`
(direct request mode); and build-helper fixup `2f1bbf1671` belongs to
`29d1f1184a` (shared Cargo and linker harness). The second build fixup,
`451610855b`, mixes a helper change with `justfile` recipe changes introduced
by `631974b745`. Keep it as a clearly named integration commit after both
prerequisites rather than forcing the whole patch onto one owner. The ten
remaining subjects target commits absent from the downstream range and have no
same-path downstream owner. Their patches form two mechanical groups by
behavior: four atomic counter calls switch from `fetch_update` to `try_update`,
and six `bail!` call sites gain terminating semicolons. Keep each group as one
clearly scoped maintenance commit rather than retaining upstream-targeted
fixup subjects.

This note records older `alpha` commits that were omitted as separate commits
during prior history rearrangements. The earlier replay onto
`deca38acd3efccd5fd5e3d79758c217da1cc952a` (0.161.0-alpha.6) is recorded
below. On 2026-10-01, upstream was force-updated from
`bdbbb05ea1f67c72e8b3835e4d38b135dccbd34b` (0.161.0-alpha.8) to
`aa3a9e0cc524cdff807fb433cca843558cbcd7d1` (0.161.0-alpha.10). The new tip no
longer contains the alpha.6 release commit, so the current merge-base is
`33aea33b39a5e35c78de75c332106479b16c6994`; `alpha` has 100 commits and
upstream has 39 commits after it. The alpha.6 version-only bump is therefore
back in the replay range and must be skipped as superseded by alpha.10. The
next replay has not started. A skipped commit is not treated as lost
functionality until its tree effect has been checked against the current
`alpha` tree.

| Skipped commit | Reason and retained behavior |
| --- | --- |
| `3cbb58fb5fa08e12032d7bcc6f1749705a32afec` — `build(android): document hybrid builds` | The actual patch adds only `codex-rs/cli/src/android_tls_alignment.rs`; despite its subject, it contains no hybrid-build documentation. The module is present in current `alpha`, owned by `bd78264325` (`fix(android): support sandbox and TLS alignment`). The effect is retained; replaying the skipped commit would duplicate it. |
| `7067c9194c21a4fb5f4700db50c1cb10bd6cb0d6` — `Reduce status indicator animation frequency` | Its patch changes the active animation cadence from 32 ms to 200 ms. Current `alpha` retains that 200 ms cap in `2aa18b576a` (`feat(tui): show wait countdown`), and `active_animation_schedules_at_most_five_frames_per_second` asserts it. The cap is intentional to limit animation CPU use on Termux/Android and must remain. |

These are intentional non-replays of duplicate tree effects, not removals of the
Android TLS alignment or the 200 ms animation cap. Recheck this note after the
upstream rebase, because upstream may independently add or change either
behavior.

The documentation refresh was committed as a fixup to this audit document's
downstream owner and autosquashed. The unique safety checkpoint
`alpha.before-rearrange-20260930-skipped-doc-fixup` points to
`b6836933a42f9344d502bc6fce69c50061c52858`, with tree
`ac110c7b55da88a66b81874805b4ec8cb293a0f2`; immediately after that
autosquash, the `alpha` tree matched exactly. The downstream series remained
104 commits with no remaining `fixup!` commits.

## Resume-progress fixup autosquashes (2026-09-30)

The resume-progress fixup was verified to target the downstream TUI resume
progress commit, then autosquashed. A later correction to the provider-retry
path was verified to target the downstream resume-session-selection commit,
then autosquashed there. This second fixup was necessary because replaying the
later session-selection change after resolving the earlier conflict otherwise
left that retry path using the old progress lifecycle. At the end of that
autosquash sequence, the `alpha` tree matched both
`alpha.before-rearrange-20260930-resume-learning` and
`alpha.before-rearrange-20260930-provider-retry-progress` exactly:
`6bc4da5450b6b80c0d828c47b8ca23dce67ffb6e`. There were no unattached
`fixup!` commits in the downstream range at that point.

The churn review found no further high-confidence squash based only on repeated
filenames. In particular, keep the adjacent app-server resume-progress and TUI
resume-progress commits separate: together they are about 1,400 changed lines
across distinct backend/protocol and UI/estimation responsibilities, which
would make an oversized rebase unit. Repeated config, app-server, and CLI path
touches also belong to distinct feature owners unless a specific repair is
identified. Avoid consolidating them solely to reduce the commit count.

## Upstream refresh and thread-store ordering (2026-09-30)

The live upstream ref moved non-fast-forward from
`9bb9257dc953c088b8995acd1130e88b2d0d5ba8` to the `0.161.0-alpha.5` release
`0d0437be13cb836487c4b2d5704d53b1a402a99d`. The refreshed merge-base is
`d1d0e89558578f622b93702686963e67948fd84c`; there are 37 upstream commits and
105 `alpha` commits after it. The old `9bb9257d` release commit is therefore in
the replay range even though it was the former upstream tip. Its release-only
version bump is superseded by upstream `0.161.0-alpha.5`; do not replay that
stale bump over the newer version. Record its eventual skip during the rebase.

The ordinal fork-boundary commit was moved next to its thread-store pagination
and ordinal foundation. The clean starting commit was `509fc4d1c4d1cade0f1cae933c561fc85d10959b`,
and `alpha.before-rearrange-20260930-rollout-cohesion` records its tree
`1f4712df627696eb8808510ded61dcd36f71439e`. Immediately after reordering,
`alpha` had that exact tree, still had 105 commits after the merge-base, and had
no `fixup!` commits.

No downstream product feature was an obvious exact duplicate of the 37 new
upstream commits, so none was dropped on subject/path similarity alone. Recheck
the rollout/history/resume cluster, generated protocol schemas, app-server
initialization and recovery, auth/reconnect, server-authoritative TUI
permissions, and per-response usage attribution against the new upstream
behavior during replay. The three quarantined tests in `24112eb0cb` remain
present upstream without recent edits to their source files; rerun them after
replay before deciding whether their ignores are still needed.

## Current rearrangement checkpoint (2026-09-30)

Before splitting the mixed resume-selection commit, `alpha` was clean at
`5231ff52402b1f17c922c9ad3a15ad87fbbe943e` with tree
`c8d990d56b2a46c3f86c59482099bccb93236e54`. The safety branch
`alpha.before-rearrange-split-resume-progress-20260930` points to that exact
commit. The rearranged `alpha` has the same tree hash, verified with
`git diff --exit-code` against the checkpoint.

The 45-file, 1,263-addition `a6d39ce025` mixed resume-selection commit was split
into three adjacent units: `fc5a44e670` streams resume progress through the
app-server/protocol and storage path; `93909b5a99` consumes and renders that
progress in the TUI; and `c8bce5e6bc` retains provider, cwd, and session resume
selection. The three commits reproduce the original `a6d39ce025` tree exactly
(`fe5e3380360f78b3162557604e5cecf2a30a6333`). The default thread-list provider
filter change remains with provider selection because it enables resuming
threads from a provider other than the currently configured one.
The TUI drains ready `thread/resume/progress` notifications ahead of the
concurrent resume response; selecting the response first could skip buffered
rollout percentages and jump directly from the initial 5% to 35%.

The current series has 106 commits since the merge-base. Repeated touches to
upstream-present paths remain highest in `codex-rs/cli/src/main.rs` (8),
`codex-rs/config/src/config_toml.rs` (7), `codex-rs/core/src/config/mod.rs`,
`codex-rs/core/config.schema.json`, and `codex-rs/Cargo.lock` (6 each), then
`codex-rs/app-server/src/request_processors/thread_processor.rs` and
`codex-rs/app-server/src/lib.rs` (5 each). These are separate feature/API
owners (CLI modes, config additions, protocol/server features, and dependency
changes), so repeated paths alone are not a reason to combine them. The new
resume-progress split removes one mixed owner; further consolidation should be
based on specific shared behavior rather than filename counts.

The small `chore: remove stale core test imports` commit was split during the
next rearrangement into two independent one-line cleanups:
`chore(core): drop stale tool source import` and
`chore(core-tests): drop stale reasoning import`. The imports became unused for
different reasons and neither cleanup has a meaningful downstream feature
owner, so neither is a fixup. The split reproduced the original cleanup tree
exactly (`06ea8b2a44f3b8269d78659a342e1db4d953056c`). Its safety branch,
`alpha.before-rearrange-core-import-split-20260930`, and the resulting `alpha`
both have tree `8369e6eddeb18f2d6ffbf288e7ea7b04d7d0c397`.

## Pre-rebase validation (2026-09-30)

- The targeted resume-progress TUI tests passed (3/3). The full `just
  test -p codex-tui` run executed 5,588 tests and had 55 failures (49 crate
  tests and 6 integration tests), including sandbox/network environment and
  snapshot/tooling failures. The JUnit report is in the external build tree at
  `codex-rs/target/nextest/local/junit.xml`; the full suite is not green.
- After prioritizing ready progress notifications in the TUI's resume wait, the
  focused TUI request-progress test passed. The app-server integration test
  `cold_legacy_resume_streams_rollout_progress_before_its_response` also passed
  when run with the escalated shell required for bubblewrap user namespaces.
- `TMPDIR=/var/tmp CARGO_INCREMENTAL=1 CODEX_CARGO_DISABLE_SCCACHE=1
  CARGO_BUILD_JOBS=4 ./scripts/build_codex.sh --release --install flip7` was
  run from the designated external build tree and exited successfully. Its log
  is `/var/tmp/install_codex_flip7-retry-20260930.log`. The staged and installed
  binary is stamped `0.160.0-alpha.2-8cd319db89-202609300239`.
- On Flip7, `$HOME/.cargo/bin/codex --version` returned that version and
  `$HOME/.cargo/bin/codex --help` succeeded. Bare `codex` still resolves to
  Termux's older `/data/data/com.termux/files/usr/bin/codex` (0.149), because
  the remote PATH contains only `$PREFIX/bin`; no device shell configuration
  was changed. The originally referenced `/var/tmp/install_codex_flip7.log`
  was absent, so the earlier failure itself could not be attributed.
- `build/android-release/release` and
  `build/android-release/aarch64-linux-android/release` are expected Cargo
  host/target output directories, not duplicate Android binaries. The Android
  `codex` artifact is under the target-triple directory.
- The upstream rebase has not started. The current `AGENTS.md` blob matches
  `alpha.before-rearrange-sandbox-docs-agents-20260930`; its incremental-build,
  source/build-tree, `/var/tmp`, linker-lock, and actual-fixup guidance are
  present in the current `alpha` tree.

## Latest exact-tree rearrangement (2026-09-30)

The clean starting `alpha` commit was `e3cee943294ca8bdee0102c13e2acd88b173c211`
with tree `4b6f2017ddfad668d2efa4bdf939780a53190b49`. The unique safety branch
`alpha.before-rearrange-agent-guidance-20260930` points to that commit. The
one-line removal of the automatic `just fmt` instruction was a second edit to
the same root `AGENTS.md` guidance as `docs: add sccache incremental dogfooding
guidance`; it was retargeted as a true fixup and autosquashed. The result,
`alpha` at `374b641ebd99d8f310d209d0a94a1c9844284ba3`, has the exact same tree
hash, verified against the safety branch. The cleanup reduced the current
downstream series from 106 to 105 commits without changing the tree. No
downstream `fixup!` commits remain unattached.

The older `alpha.before-rearrange-20260922` ref is not the safety branch for
this exercise: its merge-base with upstream is `34e74fda0eb05ce918b6ce8778857807bc9b4a6a`,
while current `alpha` is based on `6288753b469615a0b62e8cf0dd65c6e893eb4849`.
Its raw tree therefore differs from the later, already-upstream-integrated
starting state. The fresh per-exercise checkpoint above is the authoritative
tree-preservation check for this rearrangement.

## Upstream refresh before rebase (2026-09-30)

The GitHub `latest-alpha-cli` branch moved from
`5ccfcfe0fe510784521a3328b93f1c2fccc5e7a1` (`Release 0.161.0-alpha.2`) to
`9bb9257dc953c088b8995acd1130e88b2d0d5ba8` (`Release 0.161.0-alpha.3`) with a
history rewrite. A normal fetch rejected the non-fast-forward update, so the
local `upstream/latest-alpha-cli` tracking ref was refreshed explicitly; no
local `alpha` branch or user branch was overwritten. The current branch still
shares merge-base `6288753b469615a0b62e8cf0dd65c6e893eb4849` with the fetched
upstream. No rebase has started yet.

## Downstream file-churn audit (2026-09-28)

The clean `alpha` tip audited for this rearrangement was
`c6ea2a5ad73e567ccf8dd95f9f3225a01e3d4c51`, based on
`upstream/latest-alpha-cli` at `c185f4f60988ab7b70c20858f713b68103333238`.
It contained 102 downstream commits. Of files present in that upstream tree,
85 were touched by more than one downstream commit, with 17 adjacent commit
pairs sharing an upstream file. The highest counts were `cli/src/main.rs` (8),
`config/src/config_toml.rs` (7), `core/config.schema.json` (6),
`core/src/config/mod.rs` (6), and `Cargo.lock` (6).

Repeated paths are a review trigger, not an automatic squash rule. For example,
the CLI entry point and app-server event dispatcher are shared extension points
for independently testable commands and wire events; combining those features
would make commits larger and obscure ownership. Likewise, package staging,
artifact assembly/audit, and local promotion are separate release stages, and
the source/build resolver, Cargo/linker harness, artifact tracking, and `just`
routing are separate build prerequisites. Those units should stay distinct and
near one another rather than be collapsed into oversized commits. The
standalone build orchestrator and ARMv7 workflow are each dominated by one new
script and are intentionally retained as individual feature commits.

The following ownership corrections were recorded in the 2026-09-28 audit and
applied during subsequent history rearrangements:

| Change | Logical owner and treatment |
| --- | --- |
| `e7a6321d35` symlink writable-root test correction | Valid fixup to `6848eb0688`; keep with sandbox hardening. |
| `90d6f17882` explicit user-over-tool wording | Fixup to `33e184199d`; it refines the same policy instructions. |
| `c6ea2a5ad7` regex-derived writable roots | Earlier audit: fixup to `71f9350979`. In the current `alpha` chain, that commit is absent and the feature is present in `f230ba9300`; retarget the current test fixup there. |
| `db43f2ee71` stable-branch workflow policy | Fixup to `6eab13a3e5`; it completes the branch-policy migration, not V8 artifact resolution. |
| `09fd7f6043` writer-lock fallback | Move only the `rollout/src/writer_lock.rs` hunk into the active-session-takeover owner `9ca1f8913c`; retain the remaining cross-cutting Android lock support separately. |
| `88c025719d` Android sandbox executable selection | Move only the `linux_run_main.rs` executable-selection hunk into Android sandbox owner `4879388cc4`; retain the native test-helper adaptation as a separate test-infrastructure commit. |
| `04136c5126` command completion timestamps | Combine with `641e39dd2d` successful-command grouping as one bounded completed-command presentation cluster. |
| `516efa8886` remembered resume working directory | Combine with `b69637808c` resume-session selection as one bounded resume workflow cluster; both change the same selection/restoration path. |

Other repeated config, session, sandbox, and remote-control paths are retained
as separate commits where the audit found distinct APIs or behavior owners.
That rationale should be revisited if the newer upstream replay makes any of
those changes redundant or reveals a direct dependency.

## Fixup-target audit and upstream refresh (2026-10-01)

An inventory of the nine `fixup!` commits in the downstream range found eight
valid, reachable downstream owners. `38f8553f67` was mis-targeted at
`c6ea2a5ad7`, which is absent from current `alpha` and survives only on older
audit branches. Its optional-suffix rewrite tests belong with
`f230ba9300` (`feat(config): add global writable roots`): that reachable,
post-merge-base commit contains the regex rewrite implementation. It was
retargeted to that owner and autosquashed; after the upstream replay, the owner
is `e1ceae66b3`. The other audited owners were
`16876b3f06` (daemon-installer security), `0493af66bb` (resume progress),
`2a8c3532e3` (thread-store history), `4b1961112c` (session takeover),
`518f57e60d` (atomic permission presets), `e2ed399790` (resume session
selection), and `3c1de0d1a4` (fork release/version behavior).

The provider-defaults integration-test fixup updates the picker expectation to
match the downstream behavior introduced by `e2ed399790`: resume/fork pickers
show sessions from every provider, while `--last` still follows the effective
provider. Its temp home now respects `TMPDIR`; the focused test passed 1/1.
The version-snapshot fixup removes the test-only `0.0.0` display override and
normalizes the real package version in snapshots; the focused status,
history-cell, startup-draft, and app snapshot groups passed before this audit.

## Upstream replay (2026-10-01)

Before replay, `alpha.before-rebase-20261001-autosquashed-deca38ac` pointed to
`a09dac921db38f712ddd8798b8e96449b644c415` with tree
`76a5a3c00d620b4b35f7f31796fe720ff3c96e6c`, identical to the rearrangement
checkpoint. `alpha` was then rebased onto upstream
`deca38acd3efccd5fd5e3d79758c217da1cc952a` (merge-base before replay:
`5602705c9622593785441bd6a79237bd24ee3418`).

The only explicit skip was `0d0437be13` (`Release 0.161.0-alpha.5`), a
workspace-version-only bump. It conflicted with upstream's newer
`0.161.0-alpha.6` version and had no other changes. During replay,
`18131270fe` removed repository-local Codex guidance; the downstream
`docs: add sccache incremental dogfooding guidance` commit restored `AGENTS.md`
because its exact checkpoint contents remain the project's requested local
build and test guidance.

The replay completed at `fea3a198bd731c4fb4a7198017cf60dda24eba9d` on top of
the fetched upstream tip, with 103 downstream commits remaining. The post-rebase
tree is expected to differ from the pre-rebase tree because it includes the 24
new upstream commits and omits the superseded alpha.5 release bump; compare and
review the resulting delta rather than treating it as a rearrangement mismatch.
No `fixup!` commits remain unsquashed. Post-rebase build, tests, and Flip7
install validation are still pending.

## Direct-invocation daemon startup regression (2026-10-01)

A fresh-profile `codex remote-control start` reproduced the standalone-install
error for a directly invoked CLI without a complete local package. The daemon
startup preflight required a package before reaching the existing
`backend_codex_bin()` fallback. The repair returns through that fallback for a
missing package, before creating a managed-package directory, and is a true
fixup to downstream owner `99e0bf3155` (`fix(security): disable daemon
installer updates`). It was autosquashed as `0306811468`; the resulting tree
matches `alpha.before-rearrange-20261001-standalone-fixup` exactly
(`5e7a0579f43e875c62a1439df6e451e6ed4bbfc6`). The daemon crate passed all 63
tests. A live isolated-profile `remote-control start` also succeeded on
0.161.0-alpha.6 while the original default-profile 0.160.0-alpha.2 daemon
remained alive at its original PID; the canary daemon was left running for
side-by-side verification. The subsequent rebase onto alpha.10 still needs
full build, test, and Flip7 validation.

## Upstream-file churn review before alpha.10 replay (2026-10-01)

Against merge-base `deca38acd3efccd5fd5e3d79758c217da1cc952a`, the 100-commit
series at the time of this path-count review changed 589 paths that exist in
upstream; 88 of those paths were touched by multiple commits. The highest
counts are
`cli/src/main.rs` (8), `config/src/config_toml.rs` (7), `Cargo.lock`,
`core/config.schema.json`, and `core/src/config/mod.rs` (6 each), then
`app-server/src/lib.rs` and `app-server/src/request_processors/thread_processor.rs`
(5 each).

The adjacent-overlap review found no further high-confidence fixup or squash
solely from shared filenames. The eight `cli/src/main.rs` edits belong to
separate build, release-channel, ordinal-boundary, daemon-profile, Android,
remote-auth, installer-policy, and login features. The seven config-file edits
are distinct identity, writable-root, bare-prompt, provider-override,
background-poll-bound, sandbox-diagnostics, and remote-capture settings. The
schema/config-module and app-server hotspots likewise follow those independent
API owners; the resume-progress protocol and TUI consumers have already been
split into adjacent backend, UI, and session-selection commits. The repeated
`Cargo.lock` updates belong to distinct dependency owners. Keep these units
separate and ordered by feature: combining them only to reduce path counts
would produce broader commits and make replay failures harder to attribute.

The additional fixup audit found and autosquashed the daemon standalone-start
repair into its downstream owner and the skipped-commit audit correction into
its documentation owner. A closer dependency check also found that Cargo
artifact tracking adds scripts already invoked by the shared Cargo/linker
harness; it is now a fixup within that build cluster. These three autosquashes
reduced the alpha.6-based series to 99 commits. The alpha.10 upstream rewrite
puts the alpha.6 release-only commit back into the replay range, making it 100
commits. No unattached `fixup!` commits remain in the current downstream range.
The resume-progress backend/UI/session-selection
commits and the sandbox IPC/hardening commits remain separate because each
pair crosses distinct responsibilities and exceeds roughly 800 changed lines
when combined; remote-control capture and auth are separate for the same
reason (over 2,000 combined changed lines).

## Alpha.10 history preparation refresh (2026-10-02)

The current upstream branch moved after the prior audit: fetching
`upstream/latest-alpha-cli` produced `838a0aeb33acaaaf1daf6d90dad561006a38384a`
(`Release 0.162.0-alpha.1`). The local tracking ref had still pointed at the
alpha.13 tag and Git updated it to the remote tip; the merge-base with `alpha`
remains `6b4daafdb445340e5af66f067ad4057e6ed9fd81`. The alpha.13 commit remains
available by its tag. The current downstream range is 100 commits.

The latest rearrangement checkpoint is
`alpha.before-rearrange-build-cluster-20261002` at
`064a12aa296737705557a8393a748fb1fae9fa93`, tree
`abc3f7c6ffab31c60b84ae306e56537bc2c678cc`. After rearranging, `alpha` is
`49342c1ee8f993aa77d1fb948445b2eec9bc9284` with that exact same tree. The two
valid fixups were autosquashed: remote `thread/resume` timeout handling into
`feat(app-server): stream resume progress`, and reflink-required build-tree
sync into `build: require reflinks for source sync`; no `fixup!` commits remain.
The build guidance and sync change now follow the build-tree recipe setup.

The generated `sandbox_log_path` schema entry was moved out of
`docs(core): clarify background terminal poll bounds` and into
`fix(sandbox): restore bwrap debug logging`, the commit that introduces the
setting. The reflink source-sync change keeps `scripts/rsync_git_sync.py`
because `build_codex.sh` still uses it for the separate Rusty V8 checkout with
an explicit toolchain exclusion. The Codex checkout's own build-tree sync uses
`cpto --lngit` with `CPTO_REQUIRE_REFLINKS=1`. The focused helper tests passed
4/4; the remote-resume focused app-server test passed 1/1 earlier in this
preparation. Full post-rebase tests and Flip7 install validation remain
pending.

## Rebase and validation status correction (2026-10-02)

The rebase onto `upstream/latest-alpha-cli` has since completed. The fetched
upstream tip is `838a0aeb33acaaaf1daf6d90dad561006a38384a`, which is now the
merge-base of `alpha`. The current `alpha` commit is
`c2d5e70b4c18ada039f15d5fdba0af880ee623ef`, tree
`52fd5567ab72a09a31a9aad50ebbe39bae1a63be`. That tree exactly matches the
unique safety checkpoint `alpha.before-rearrange-20261002-postrebase-fixups`
at `f8f15f02436123cae41a1e8ae8d92d8142a80ae5` (same tree hash). The earlier
`alpha.before-rebase-20261002-alpha1` checkpoint remains unchanged at commit
`a5bc261ea66807d460922120d321f04d279f7eca`, tree
`53ea27d942b487539627400509e8c8f7eb825b3f`. `alpha` currently contains 100
commits after the upstream tip, with no unsquashed `fixup!`, `squash!`, or
`amend!` subjects in that range.

The schema-export regeneration is now attached to the downstream config-reload
feature, and nested `.git` cleanup is attached to the source/build-tree fix.
The TUI's incompatible-daemon recovery dialog originated upstream in
`d6093d3228`; downstream `71703cf898` adapts it by allowing embedded fallback
for unmanaged daemons. It is intentionally a separate downstream compatibility
change, not a fixup to an upstream commit; no downstream owner for this
behavioral adaptation was identified. Its focused real-TUI integration test
`incompatible_daemon_falls_back_for_default_and_explicit_features` passed
1/1 in an isolated user/mount namespace, confirming that an unmanaged daemon
feature mismatch falls back without presenting the unavailable restart
action. The installed TUI binary itself has not yet been reverified.
The app-server-protocol package passed 319 tests, and app-server-daemon passed
70 tests. The first full workspace test run exposed two compile errors; the
restored upstream bwrap test helper and missing Unix `PermissionsExt` import
are local uncommitted corrections. The first focused Linux-sandbox run then
compiled and ran 240 tests: 217 passed, 23 failed. Failures include stale
physical-versus-logical temporary-path expectations and sandbox prerequisites
physical-versus-logical temporary-path expectations. After adjusting those
expectations, isolating the login-shell test from the host `.bashrc`, and
updating the `.aws` assertion to match upstream commit `1d804e91b7`, the full
Linux-sandbox package passed in an isolated user/mount namespace: 240 passed,
3 skipped. The `.aws` expectation is separate test maintenance because its
implementation owner is upstream. The symlinked-temp integration fixture now
keeps its alias in the writable workspace and its redirected target below the
authorized temp root; both its focused test and the full package passed. An
ordinary nested-sandbox invocation of the TUI integration test failed before
assertions with `Operation not permitted`; the isolated-namespace retry passed.
An isolated bubblewrap mountpoint probe under the physical `/var/tmp` target
left its disposable directory unchanged. No cleanup was performed on the
shared socket directory.

The working tree currently contains uncommitted sandbox-test adjustments and
the two compile corrections listed above. Full `just test`, release build, and
`build_codex.sh --release --install flip7` validation remain outstanding; do
not treat this rebase as fully validated.
After those adjustments, `cargo check --locked --tests -p codex-linux-sandbox`
completed successfully from the external build tree (compile-only; this was
followed by the passing full package test run recorded above).

## Auth and command-group validation repairs (2026-10-02)

Two focused repairs were committed as actual downstream fixups and autosquashed.
Auth account-context changes now notify remote-control consumers even when
refresh credentials compare equal. This belongs to the split remote-control
credentials feature (formerly `4048a85b11`, now `451acb5846`). Its enrollment
regression test passed; the test waits 900 ms, below the one-second retry delay,
to verify notification-driven enrollment without the previous 100 ms load flake.

Exploration groups retain failed reads and following reasoning while staying
separate from compact ordinary command groups. Replay flushes a completed
ordinary group before a failure, but preserves exploration grouping. These
repairs belong to successful command grouping (formerly `42c73dd12d`, now
`1c39261b9a`). Five focused grouping/replay tests passed. The snapshot removes
variable completion timestamps while retaining command durations.

Before autosquashing, the clean branch was checkpointed as
`alpha.before-rearrange-auth-tui-fixups-20261002`, commit
`3911fb08a0d4be64e93f9f95b4eec7fc874426ae`, tree
`616a577cf1fa717e18059fb73d88057cc376eb7a`. The resulting branch was
`376f5a6b3f9d618f60198b79bf8c9feca9bdcf3d`, with exactly the same tree;
`git diff --exit-code` also confirmed equality. No fixup subjects remained.

The elevated full TUI run before the final focused replay repair had 5,652
passes, 35 failures, two timeouts, and eight skips. Its log is
`/var/tmp/codex-alpha-20261002-tui-full-escalated.log`. Several remaining
command-history tests assume immediate flushing and need investigation against
the downstream deferred-grouping contract. Broader failures, full workspace
validation, installed TUI verification, and the Flip7 release install remain
outstanding. This record does not claim final validation.

The follow-up command-history validation passed all 59 selected tests without
retries (`/var/tmp/codex-alpha-20261002-command-history-validation-retry.log`).
Successful-command tests now check deferred grouping before flushing at a
boundary. Completion timestamp normalization occurs before live/replay equality
and snapshot comparisons; command durations and ordering remain asserted.
An earlier follow-up process terminated with signal 15 during synchronization,
before compilation, and was confirmed absent before this successful retry.

A fresh upstream fetch now points to `a986d89b99bc2a5556f3f757a6b8d8b4c6711d1f`
(`Release 0.162.0-alpha.5`). The remote tracking ref moved non-fast-forward from
`838a0aeb33acaaaf1daf6d90dad561006a38384a`; use that previous upstream boundary
explicitly when replaying downstream commits onto the new tip. The new
`chore(scripts): add rollout model updater` commit was added independently during
validation and is preserved.

## Alpha.5 checkpoint and replay (2026-10-02)

The clean branch was checkpointed before the final command-history autosquash as
`alpha.before-rearrange-command-history-20261002`, commit
`ec2eb6ef57f616369b1c8db29e0eb241037706dc`, tree
`92b54e6250127344b2e6dc4e7e5f676f04c0eca2`. The rearranged commit
`cb757b4a3409ef4683b261554eb6db5a7b5ddc77` has exactly that tree, verified by
tree hashes and an empty `git diff --exit-code`. No fixups remained.

The distinct pre-rebase backup `alpha.before-rebase-20261002-alpha5` preserves
that rearranged commit and tree. All 101 downstream commits were rebased from
the explicit previous upstream boundary `838a0aeb33` onto `a986d89b99`
without conflicts or skipped commits. The resulting commit was
`7fdc965221256ccf89c6aa2998d9e8bedc593b3f`, tree
`8e2085c09f41bd6397812514f7552501a048333e`; its merge-base is the new upstream
tip. Every entry in the pre/post rebase range-diff was equivalent. Tree equality
was required and proved for rearrangement; the upstream rebase intentionally
changes the tree by incorporating upstream changes.

Full post-rebase TUI validation is running with output in
`/var/tmp/codex-alpha-20261002-alpha5-tui-validation.log`. Two CWD test
expectations are corrected locally to preserve logical temporary paths. Their
implementation and old expectations both originated upstream in `b114781495f`,
so this test maintenance has no downstream fixup owner. It must remain a scoped
maintenance commit if retained. No production CWD behavior was changed.
Full workspace tests, local release validation, and Flip7 installation are
still outstanding.

The alpha.5 full TUI retry completed with 5,669 passes, 27 failures, two timeouts,
and eight skips (5,698 tests). Both corrected logical-CWD tests passed, as did
the command-history grouping tests. The log is
`/var/tmp/codex-alpha-20261002-alpha5-tui-validation-retry.log`. Remaining
failures include account-policy refresh after persisted account changes,
app-state/permission expectations, terminal rendering, snapshots, and reconnect
timeouts. Several tests aborted and need their actual error inspected before
classification. This run compiled the rebased TUI successfully but does not
prove complete validation. CWD test maintenance was committed separately as
`d02e50faff`; its implementation and stale expectations originated upstream.
