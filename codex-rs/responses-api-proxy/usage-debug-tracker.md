# Responses API Proxy Usage Debug Tracker

This note tracks which changes helped the `codex-responses-api-proxy` usage-accounting path and which ones have not yet produced a `usage-*.log` entry in the Hermes end-to-end path.

## Helped

- `--auth-file` is now the source of truth for both upstream auth and accounting identity when the proxy is launched with that flag.
- Proxy requests to the Codex backend are normalized to force `store=true` and `stream=true`, and Hermes-only `extra_headers` are stripped before forwarding.
- SSE completions now backfill usage with `GET /responses/{response_id}` when the inline completion event omits usage.
- Buffered / non-SSE completions now also backfill usage with `GET /responses/{response_id}` when the initial JSON completion omits usage.
- The shared usage logger now labels proxy writes with `pid≈` so proxy log lines are easy to distinguish from app-server lines.
- The proxy binary can be launched through the Hermes wrapper and reaches the backend through the configured upstream proxy.

## Not Yet Proven To Help

- Setting `CODEX_USAGE_LOG_DIR` only in the Hermes wrapper has not yet resulted in a new `usage-*.log` file in the isolated test directory.
- The `pid≈` log prefix is only a label change; it does not by itself cause a log file to be created.
- Matching the Hermes call to the proxy PID in the usage log has not yet been demonstrated in the isolated one-shot repro because the log file is still not appearing.

## Current Status

- Hermes can reach the local `codex-responses-api-proxy`.
- The proxy can reach the backend.
- The proxy still is not producing a visible usage log file in the isolated `CODEX_USAGE_LOG_DIR` repro.

## Next Checks

- Confirm whether the proxy process itself inherits `CODEX_USAGE_LOG_DIR`.
- Confirm whether `record_account_token_usage()` is reached for the Hermes request path.
- Enable proxy-side debug usage logging and inspect the exact completion/backfill path taken for a single Hermes turn.
