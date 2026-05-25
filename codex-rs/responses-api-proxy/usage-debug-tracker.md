# Responses API Proxy Usage Debug Tracker

This note tracks which changes helped the `codex-responses-api-proxy` usage-accounting path and which ones did not.

## Helped

- `--auth-file` is now the source of truth for both upstream auth and accounting identity when the proxy is launched with that flag.
- Proxy requests to the Codex backend preserve `store=false`, force `stream=true`, and strip Hermes-only `extra_headers` before forwarding.
- SSE completions now backfill usage with `GET /responses/{response_id}` when the inline completion event omits usage.
- Buffered / non-SSE completions now also backfill usage with `GET /responses/{response_id}` when the initial JSON completion omits usage.
- The shared usage logger now labels proxy writes with `pid≈` so proxy log lines are easy to distinguish from app-server lines.
- The proxy binary can be launched through the Hermes wrapper and reaches the backend through the configured upstream proxy.
- The proxy classifies Codex `/responses` replies as SSE when the normalized request body has `stream=true`, even if the backend omits `Content-Type`.
- A direct replay of a captured Hermes `/responses` body now produces `usage-infinityresonance@gmail.com.log` and writes usage with the proxy PID.
- A live Hermes one-shot now writes the same usage log file under an isolated `CODEX_USAGE_LOG_DIR`.

## Did Not Help

- The temporary local-probe shortcut for `/backend-api/codex/models` and related discovery endpoints did not fix the accounting issue and was removed.
- Rewriting `store=false` to `store=true` did not fix the issue and was removed.
- The proxy-side debug-only logging added during investigation was not needed for the final fix and was removed.

## Current Status

- Hermes reaches the local `codex-responses-api-proxy`.
- The proxy reaches the backend and now treats Codex `/responses` output as SSE when the request asks for streaming.
- The proxy writes `usage-infinityresonance@gmail.com.log` in isolated repros and in the live Hermes one-shot.
- The `pid≈` prefix is in place so proxy log lines are easy to distinguish from app-server lines.

## Next Checks

- Keep the cleanup small: no further accounting-path changes are currently required.
