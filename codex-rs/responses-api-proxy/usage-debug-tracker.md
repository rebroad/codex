# Responses API Proxy Usage Debug Tracker

This note tracks which changes helped the `codex-responses-api-proxy` usage-accounting path and which ones did not.

## Helped

- The proxy now loads prompt-debug settings through `codex-core`'s config builder, so it uses the same config source and precedence as the app-server for `prompt_debug_http.capture_dir`.
- `--auth-file` is now only the source of upstream auth; the proxy’s accounting identity follows the active Codex account so it matches app-server and `codex status`.
- Proxy requests to the Codex backend preserve `store=false`, force `stream=true`, and strip Hermes-only `extra_headers` before forwarding.
- SSE completions now backfill usage with `GET /responses/{response_id}` when the inline completion event omits usage.
- Buffered / non-SSE completions now also backfill usage with `GET /responses/{response_id}` when the initial JSON completion omits usage.
- The shared usage logger now labels proxy writes with `pid≈` so proxy log lines are easy to distinguish from app-server lines.
- The proxy binary can be launched through the Hermes wrapper and reaches the backend through the configured upstream proxy.
- The proxy classifies Codex `/responses` replies as SSE when the normalized request body has `stream=true`, even if the backend omits `Content-Type`.
- A direct replay of a captured Hermes `/responses` body now produces `usage-infinityresonance@gmail.com.log` and writes usage with the proxy PID.
- A live Hermes one-shot now writes the same usage log file under an isolated `CODEX_USAGE_LOG_DIR`.
- The `.query_id_counter` file now lives under the prompt-debug capture dir chosen by the shared Codex config, which keeps proxy query IDs aligned with app-server captures.
- A live Hermes one-shot now writes `query_id=938` while the shared counter advances to `939`, proving the proxy is using the same active-account capture bucket as app-server and `codex status`.
- The same live one-shot also updates `codex status` from `usage 18.24%` to `usage 18.36%`, which shows the proxy is writing back the shared backend usage state instead of a private side bucket.

## Did Not Help

- Pointing `CODEX_USAGE_LOG_DIR` at the prompt-debug capture dir was the wrong abstraction and was removed.
- The temporary local-probe shortcut for `/backend-api/codex/models` and related discovery endpoints did not fix the accounting issue and was removed.
- Rewriting `store=false` to `store=true` did not fix the issue and was removed.
- The proxy-side debug-only logging added during investigation was not needed for the final fix and was removed.

## Current Status

- Hermes reaches the local `codex-responses-api-proxy`.
- The proxy reaches the backend and now treats Codex `/responses` output as SSE when the request asks for streaming.
- The proxy writes usage logs under `CODEX_USAGE_LOG_DIR`, while the query-id counter and capture files follow the prompt-debug config source.
- The proxy and `codex status` now share the same active account bucket, so backend `used_percent` transitions are de-duplicated in the shared store.
- The `pid≈` prefix is in place so proxy log lines are easy to distinguish from app-server lines.

## Next Checks

- Keep the cleanup small: no further accounting-path changes are currently required.
