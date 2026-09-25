# codex-linux-sandbox

This crate is responsible for producing:

- a `codex-linux-sandbox` standalone executable for Linux that is bundled with the Node.js version of the Codex CLI
- a lib crate that exposes the business logic of the executable as `run_main()` so that
  - the `codex-exec` CLI can check if its arg0 is `codex-linux-sandbox` and, if so, execute as if it were `codex-linux-sandbox`
  - this should also be true of the `codex` multitool CLI

On Linux, Codex prefers the first `bwrap` found on `PATH`
outside the current working directory whenever it is available. If `bwrap` is
present but too old to support
`--argv0`, the helper keeps using system bubblewrap and switches to a
no-`--argv0` compatibility path for the inner re-exec. If `bwrap` is missing,
the helper falls back to the bundled `codex-resources/bwrap` binary shipped
with Codex.
Codex also surfaces a startup warning when `bwrap` is missing so users know it
is falling back to the bundled helper. Codex surfaces the same startup warning
path when bubblewrap cannot create user namespaces. WSL2 follows the normal
Linux bubblewrap path. WSL1 is not supported for bubblewrap sandboxing because
it cannot create the required user namespaces, so Codex rejects sandboxed shell
commands that would enter the bubblewrap path.

**Current Behavior**
- Legacy `SandboxPolicy` / `sandbox_mode` configs remain supported.
- Bubblewrap is the default filesystem sandbox.
- If `bwrap` is present on `PATH` outside the current working directory, the
  helper uses it.
- If `bwrap` is present but too old to support `--argv0`, the helper uses a
  no-`--argv0` compatibility path for the inner re-exec.
- If `bwrap` is missing, the helper falls back to the bundled
  `codex-resources/bwrap` path.
- If `bwrap` is missing, Codex also surfaces a startup warning instead of
  printing directly from the sandbox helper.
- If bubblewrap cannot create user namespaces, Codex surfaces a startup warning
  instead of waiting for a runtime sandbox failure.
- WSL2 uses the normal Linux bubblewrap path.
- WSL1 is not supported for bubblewrap sandboxing; Codex rejects sandboxed
  shell commands that would require the bubblewrap path before invoking `bwrap`.
- Filesystem-restricted execution requires bubblewrap. The legacy Landlock
  option is rejected for these policies because it cannot isolate app-server
  Unix sockets. Disable `features.use_legacy_landlock` when upgrading.
- Split-only filesystem policies that do not round-trip through the legacy
  `SandboxPolicy` model stay on bubblewrap so nested read-only or denied
  carveouts are preserved.
- When bubblewrap is active, the helper applies `PR_SET_NO_NEW_PRIVS` and a
  seccomp network filter in-process.
- When bubblewrap is active, the filesystem is read-only by default via `--ro-bind / /`.
- When bubblewrap is active, writable roots are layered with `--bind <root> <root>`.
- When bubblewrap is active, protected subpaths under writable roots (for
  example `.git`,
  resolved `gitdir:`, and `.codex`) are re-applied as read-only via `--ro-bind`.
- When bubblewrap is active, overlapping split-policy
  entries are applied in path-specificity order so narrower writable children
  can reopen broader read-only or denied parents while narrower denied subpaths
  still win. For example, `/repo = write`, `/repo/a = none`, `/repo/a/b = write`
  keeps `/repo` writable, denies `/repo/a`, and reopens `/repo/a/b` as
  writable again.
- When bubblewrap is active, unreadable glob entries are expanded before
  launching the sandbox and matching files are masked in bubblewrap:

  ```text
  Prefer:   rg --files --hidden --no-ignore --glob <pattern> -- <search-root>
  Fallback: internal globset walker when rg is not installed
  Failure:  any other rg failure aborts sandbox construction
  ```

  Users can cap the scan depth per permissions profile:

  ```toml
  [permissions.workspace.filesystem]
  glob_scan_max_depth = 2

  [permissions.workspace.filesystem.":workspace_roots"]
  "**/*.env" = "none"
  ```

- When bubblewrap is active, symlink-in-path and non-existent protected paths inside
  writable roots are blocked by mounting `/dev/null` on the symlink or first
  missing component.
- When bubblewrap is active, the helper explicitly isolates the user namespace via
  `--unshare-user`. By default it also creates a PID namespace via `--unshare-pid`.
- When bubblewrap is active and network is restricted without proxy routing, the helper also
  isolates the network namespace via `--unshare-net`.
- In that isolated restricted mode, IPv4/IPv6 socket operations are available for local IPC
  within the sandbox namespace, while host-network access and filesystem Unix sockets remain
  blocked.
- In managed proxy mode, the helper uses `--unshare-net` plus an internal
  TCP->UDS->TCP routing bridge so tool traffic reaches only configured proxy
  endpoints.
- In managed proxy mode, after the bridge is live, seccomp blocks new
  AF_UNIX/socketpair creation for the user command.
- When bubblewrap is active, it mounts a fresh `/proc` via `--proc /proc` by default.
  If that mount is denied, it retains the inherited `/proc` and still creates a
  PID namespace, preserving the existing fallback. `--no-proc` also retains the
  inherited `/proc` without disabling PID isolation. In these cases, process IDs
  inside the sandbox can differ from those exposed by `/proc`. Default invocations
  send no new helper flags and remain compatible with older helpers.
- Trusted provisioning of a dedicated environment can start
  `codex exec-server --linux-sandbox-pid-namespace=inherit`. This startup-only
  setting applies to both process and filesystem helpers; repository config and
  command environment variables cannot enable it. The helper receives the new
  `--inherit-pid-namespace` option, which requires an updated helper; deploy the
  helper and startup flag together.
  Inheritance reuses the caller's PID namespace and `/proc` together, omitting
  `--unshare-pid` and `--as-pid-1`. This preserves process lookups but allows
  sandboxed commands to signal other same-UID processes, including the executor.
  With `:minimal`, the existing `/proc` is bound read-only, preserving its
  container masks; explicit filesystem denials are applied afterward.
  Filesystem, user, IPC, network, seccomp, and existing container `/proc` masks
  remain in force. The default `isolate` mode retains PID isolation.

**How the Linux sandbox command is assembled (developer notes)**

The `codex-linux-sandbox` invocation and the `bwrap` invocation are separate
layers. The first passes policy and command data to Codex's helper; it is not
the final mount list. The construction path is:

1. The caller resolves the active permission profile into runtime filesystem
   and network policies. When Codex constructs a profile from configuration,
   `additional_writable_roots` rewrite rules are applied to the original roots
   at this stage; their results must be absolute, concrete paths. A profile
   supplied as an explicit runtime override bypasses that construction path,
   so its producer must supply concrete roots. Rewrite expressions are not
   permission-profile paths or bwrap paths.
2. `codex-rs/sandboxing/src/landlock.rs` serializes the runtime permission
   profile as JSON and passes it to the helper with `--permission-profile`,
   together with the policy cwd, command cwd, and command after `--`.
3. `codex-rs/linux-sandbox/src/linux_run_main.rs` parses that input, obtains
   the filesystem and network policies, selects the bwrap options, and
   assembles the inner command that reapplies seccomp after bwrap has created
   the filesystem view.
4. `codex-rs/linux-sandbox/src/bwrap.rs` turns the concrete filesystem policy
   into ordered mounts. In broad terms it establishes a read-only filesystem
   baseline (or a minimal tmpfs baseline), mounts readable roots, binds
   writable roots, and then reapplies protected metadata and denied-path
   masks. Distinct logical writable roots that resolve through symlinks to the
   same physical target share one writable bind; each root's carveouts are
   still applied, and separate nested writable roots retain their normal
   ordering. It adds namespace, proc, cwd, and command arguments around that
   mount plan.
5. The helper executes the selected system or bundled `bwrap` with that argv.
   Synthetic mount-target preparation and cleanup are helper-side setup; they
   are not permission roots or additional command-line grants.

When diagnosing a captured command, inspect the two argv layers separately:
the inner `codex-linux-sandbox --permission-profile <JSON> ...` arguments show
the runtime policy input, while the outer `bwrap ...` arguments show the
concrete filesystem view. A rewrite expression appearing as a literal path in
the runtime profile indicates it was not expanded by the profile's producer;
the bwrap builder does not interpret rewrite syntax. Exact duplicate profile
entries may be redundant input, while logical aliases can be distinct policy
entries that map to one bind target. Repeated daemon-socket mask mounts can be
intentional: the helper reapplies a mask after each bind that would otherwise
expose that directory again (for example, binding `/tmp` after masking a
socket directory beneath it).

**Notes**
- The CLI surface is `codex sandbox`; the host OS selects the sandbox backend.
