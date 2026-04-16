# Configuration

For basic configuration instructions, see [this documentation](https://developers.openai.com/codex/config-basic).

For advanced configuration instructions, see [this documentation](https://developers.openai.com/codex/config-advanced).

For a full configuration reference, see [this documentation](https://developers.openai.com/codex/config-reference).

## Connecting to MCP servers

Codex can connect to MCP servers configured in `~/.codex/config.toml`. See the configuration reference for the latest MCP server options:

- https://developers.openai.com/codex/config-reference

## MCP tool approvals

Codex stores per-tool approval overrides for custom MCP servers under
`mcp_servers` in `~/.codex/config.toml`:

```toml
[mcp_servers.docs.tools.search]
approval_mode = "approve"
```

## Apps (Connectors)

Use `$` in the composer to insert a ChatGPT connector; the popover lists accessible
apps. The `/apps` command lists available and installed apps. Connected apps appear first
and are labeled as connected; others are marked as can be installed.

## Notify

Codex can run a notification hook when the agent finishes a turn. See the configuration reference for the latest notification settings:

- https://developers.openai.com/codex/config-reference

When Codex knows which client started the turn, the legacy notify JSON payload also includes a top-level `client` field. The TUI reports `codex-tui`, and the app server reports the `clientInfo.name` value from `initialize`.

## JSON Schema

The generated JSON Schema for `config.toml` lives at `codex-rs/core/config.schema.json`.

## SQLite State DB

Codex stores the SQLite-backed state DB under `sqlite_home` (config key) or the
`CODEX_SQLITE_HOME` environment variable. When unset, WorkspaceWrite sandbox
sessions default to a temp directory; other modes default to `CODEX_HOME`.
Per-account token usage aggregates and backend usage reset history are stored
in a separate usage database under `sqlite_home` (see `docs/account-usage.md`).
This usage DB is updated whenever token usage or rate limit snapshots are
received, including during ephemeral sessions.

## Account Usage Estimator

You can tune account usage percent estimation with `[account_usage_estimator]`:

```toml
[account_usage_estimator]
min_usage_pct_sample_count = 1
max_usage_pct_display_percent_before_full = 101.0
stable_backend_percent_window = 5
```

Knobs:

- `min_usage_pct_sample_count`: minimum `account_usage_samples` rows required before `usage_pct[...]` is shown.
- `max_usage_pct_display_percent_before_full`: cap applied to predicted usage percentages while backend `used_percent < 100`; no cap is applied once backend usage reaches 100.
- `stable_backend_percent_window`: rolling window size used to compute stabilized backend percent.

## Sandbox Debug Logging

Set `sandbox_debug = false` in `config.toml` to disable Linux sandbox debug
logging written to `/tmp`. This sets `CODEX_SANDBOX_DEBUG=0` for the sandbox
runtime.

## Custom CA Certificates

Codex can trust a custom root CA bundle for outbound HTTPS and secure websocket
connections when enterprise proxies or gateways intercept TLS. This applies to
login flows and to Codex's other external connections, including Codex
components that build reqwest clients or secure websocket clients through the
shared `codex-client` CA-loading path and remote MCP connections that use it.

Set `CODEX_CA_CERTIFICATE` to the path of a PEM file containing one or more
certificate blocks to use a Codex-specific CA bundle. If
`CODEX_CA_CERTIFICATE` is unset, Codex falls back to `SSL_CERT_FILE`. If
neither variable is set, Codex uses the system root certificates.

`CODEX_CA_CERTIFICATE` takes precedence over `SSL_CERT_FILE`. Empty values are
treated as unset.

The PEM file may contain multiple certificates. Codex also tolerates OpenSSL
`TRUSTED CERTIFICATE` labels and ignores well-formed `X509 CRL` sections in the
same bundle. If the file is empty, unreadable, or malformed, the affected Codex
HTTP or secure websocket connection reports a user-facing error that points
back to these environment variables.

## Backend Query Capture

Set `[prompt_debug_http]` in `config.toml` to capture backend traffic and query payloads as files:

```toml
[prompt_debug_http]
enabled = true
capture_input = true
capture_output = true
capture_reasoning = true
capture_dir = "/tmp"
```

If `enabled = true`, capture files are written under `capture_dir` (defaults to `/tmp`).
The environment variables `CODEX_BACKEND_CAPTURE`, `CODEX_BACKEND_CAPTURE_INPUT`,
`CODEX_BACKEND_CAPTURE_OUTPUT`, `CODEX_BACKEND_CAPTURE_REASONING`, and
`CODEX_BACKEND_CAPTURE_DIR` can force-enable or override runtime capture settings.
Capture files include:
- a full backend traffic stream: `backend_traffic.ndjson`
- query-id scoped files, for example:
`codex_backend_query_<query_id>.input.md`,
`codex_backend_query_<query_id>.output.md`, and
`codex_backend_query_<query_id>.reasoning.md`.
If the `capture_dir` path contains `$$`, Codex replaces it with the current
process PID.

For a one-off run, `codex exec --debug "..."`
force-enables capture for that invocation using the same backend capture settings
and file naming.

## App-server logging

Codex app-server logs default to `stderr`. You can redirect them to a file or
duplicate them to both stderr and a file via `[app_server_log]`:

```toml
[app_server_log]
mode = "log_and_stderr"
log_file = "/absolute/path/to/codex-app-server.log"
```

Valid modes are `stderr` (default), `log`, and `log_and_stderr`. When `mode`
includes `log` and `log_file` is omitted, Codex writes to
`~/.codex/log/codex-app-server.log` (or the directory set by `log_dir`).
If the `log_file` path contains `$$`, Codex replaces it with the current
process PID (for example, `"/tmp/codex-app-server-$$.log"` -> `"/tmp/codex-app-server-12345.log"`).

## Bare prompt mode

Set `bare_prompt = true` in `config.toml` to disable built-in/system and
contextual prompt scaffolding so only user text is sent.

```toml
bare_prompt = true
```

You can also enable this for a single `codex exec` invocation with
`--bare-prompt`.

## Built-in Tool Policy

Use `builtin_enabled_tools` and `builtin_disabled_tools` in `config.toml` to
control built-in (non-MCP) tools:

```toml
# Allow-list mode: only these built-in tools are exposed.
builtin_enabled_tools = ["update_plan", "view_image"]

# Optional deny-list applied after the allow-list.
builtin_disabled_tools = ["view_image"]
```

Notes:

- If `builtin_enabled_tools` is unset, built-in tools are allowed by default
  (subject to other feature/model constraints).
- If `builtin_enabled_tools = []`, all built-in tools are blocked.
- `builtin_disabled_tools` always wins over `builtin_enabled_tools`.
- This policy applies only to built-in tools; MCP tools are configured through
  `mcp_servers`.

## Project workspace writable roots

For projects using `workspace-write` sandbox mode, you can import extra
writable roots from a VS Code `.code-workspace` file:

```toml
[projects."/home/rebroad/src/codex"]
trust_level = "trusted"
workspace_file = "/home/rebroad/Desktop/codex.code-workspace"
```

Codex reads the workspace file's `folders[].path` entries and adds them to
the sandbox writable roots for that project.

## Prompt layers and one-off overrides (`codex exec`)

Canonical terms used by Codex:

- `base instructions`: the core model instructions (from built-ins, optionally overridden by `model_instructions_file` or `instructions`).
- `developer instructions`: additional higher-priority guidance layered as a `developer` message.
- `personality`: a style preset (`none|friendly|pragmatic|comedic`) applied by supported models.

`system` in `codex exec` is a legacy alias for `developer instructions`.

`~/.codex/config.toml` keys for these layers:

- `model_instructions_file = "/abs/path/file.md"`: file-based base-instructions override.
- `instructions = "..."`: inline base-instructions override.
- `developer_instructions = "..."`: inline developer-instructions override.
- `personality = "pragmatic"` (or `none|friendly|comedic`).

For per-run overrides without editing `~/.codex/config.toml`, `codex exec` accepts
file-path based flags (files contain the text):

- `--developer-instructions-file FILE` (alias: `--system`)
- `--base-instructions-file FILE` (alias: `--base-instructions`)
- `--personality none|friendly|pragmatic|comedic`
- `--compact-prompt-file FILE` (alias: `--compact-prompt`)
- `--compact-summary-preamble-file FILE`

Compaction has two conceptual phases:

- `before compaction` prompt (departing context window): configurable via `compact_prompt` / `--compact-prompt-file`.
- `after compaction` summary preamble (arriving context window): configurable via `compact_summary_preamble` / `--compact-summary-preamble-file`.

## Notices

Codex stores "do not show again" flags for some UI prompts under the `[notice]` table.

## Plan mode defaults

`plan_mode_reasoning_effort` lets you set a Plan-mode-specific default reasoning
effort override. When unset, Plan mode uses the built-in Plan preset default
(currently `medium`). When explicitly set (including `none`), it overrides the
Plan preset. The string value `none` means "no reasoning" (an explicit Plan
override), not "inherit the global default". There is currently no separate
config value for "follow the global default in Plan mode".

## Compaction mode

By default, OpenAI-backed sessions use the remote compact endpoint
(`responses/compact`) for context compaction.

To force local compaction instead, set this feature flag in `config.toml`:

```toml
[features]
local_compaction = true
```

When `local_compaction = true`, Codex uses the local compaction flow (the
normal prompt + summary generation path) for both automatic compaction and
manual `/compact`, even when the provider would otherwise use remote compact.

## Realtime start instructions

`experimental_realtime_start_instructions` lets you replace the built-in
developer message Codex inserts when realtime becomes active. It only affects
the realtime start message in prompt history and does not change websocket
backend prompt settings or the realtime end/inactive message.

Ctrl+C/Ctrl+D quitting uses a ~1 second double-press hint (`ctrl + c again to quit`).
