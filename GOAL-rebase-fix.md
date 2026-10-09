we still have issues with this latest rebased codex. Firstly, when I run `codex remote-control start` it actually starts an app-server using the path pointed to by `~/.codex/packages/standalone/current/codex` rather than itself - I want it to use itself! Secondly, it is failing to `codex resume` (when no app-server is running), I tried to resume this session using our latest downstream codex, but it failed with `Error: times out discarding buffered terminal output` so this is a regression. Also, with `codex.chatgpt` and `codex` (our latest downstream), I am still seeing this:-

```
  Background server has incompatible feature settings
  This session requires api_key_model_discovery to be disabled
  Restart will use these shared feature settings:
    api_key_model_discovery = false
    auth_elicitation = true
    code_mode_host = true
    mcp_oauth_refresh_coordination = false
  These settings persist and can disable functionality for other clients. Restart may interrupt active or queued work.


  1. Run without daemon this time
  2. Restart with these settings
› 3. Cancel
```
why? Can we fix this somehow?

Also, I notice in our latest rebased version, we seem to have lost some of the rust-release.yml changes we had (e.g. replacing "Update latest-alpha-cli branch" with "Update alpha branch" specifically for my `reb.ai/codex/install.sh` URLs/etc.

Also, commit 760ede5a9ff856b1068e54cb3e97546a7577174a is quite possibly wrong - we should not be removing the upstream addition of the `compression::read_rollout_lines` function, should we? Or should we? If we should this needs to be clearly why explained in `DOWNSTREAM.md` otherwise I'm assuming it's a mistake.

Also, I am not sure commit `2178ffe115c784e5e356157225437d6afbd1ed0f` was the right approach - perhaps clarify the pros and cons of doing this in `DOWNSTREAM.md` and I'll review it there (in that doc).

Do we need the `build_test_cli_if_needed.sh` script introduced in commit 6dc1af5e59? If so, re-add it.

Also, can we find out why `/usr/lib/chatgpt/resources/codex -c features.code_mode_host=true app-server --analytics-default-enabled` fails to start (for the ChatGPT desktop app) when there is already an app-server running?

Finally, fix the ordinary shell's `/var/tmp` write access. Although `/var/tmp` is configured as an additional writable root, it resolves (via a symlink) to `/mnt/kingston/@/var/tmp`, and ordinary-shell creation currently fails with `Read-only file system`. Diagnose the effective mount/sandbox path mapping, then verify ordinary-shell create and remove operations under `/var/tmp`. I.e. if the defined root is a symlink, then it should resolve such that the destination of the symlink is a writable-root (unless the symlink itself is within a writeable-root, in which case it should be excluded).

Also fix `codex resume` when the requested rollout is already active in another session: it currently fails with `failed to take over the active session`. It should connect to the already-active session instead of trying to take it over. Reproduce this with the provided rollout path and verify the resumed client attaches to the existing session without taking ownership away from it.

Finally, fix `codex exec --bare-prompt 'say hello'` so it still creates and updates a rollout file. Only the explicit `--direct` option should bypass rollout creation. Verify both modes: bare-prompt persists the rollout, while direct mode does not.

Also fix the bare-prompt token-usage regression: `codex exec --bareprompt 'say hi'` must use bare-prompt semantics and report local token usage below 20.0 for the `say hi` prompt. Verify that it does not inject the default Codex instructions or context.

Also diagnose why test runs spawn repeated `git fetch` processes for the OpenAI plugins repository. The current source shows curated-plugin startup synchronization invoking a depth-one Git fetch; identify which tests exercise that path and why they reach the real remote instead of a fixture. This runtime refresh is not a build prerequisite, so keep ordinary builds/tests hermetic and avoid redundant network fetches. Ensure child fetch processes terminate when their owning test process exits, and ensure they cannot inherit and retain the Cargo target lock after the owner exits. A test-spawned fetch must not remain orphaned and block later builds.

RESOLVED: Android ChatGPT remote-control and `codex resume` both failed while the active app-server was the upstream binary. After restarting it with the downstream `codex` binary, Android remote-control connected to the ongoing session and `codex resume` worked. The two reported failures had the same cause: the running server did not contain the downstream session reconnect changes. Confirmed by the live app-server process executable `/home/rebroad/.cargo/bin/codex-0.162.0-alpha.20-3b690898a5+202610091517` and by both successful client connections after restart.
