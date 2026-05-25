# codex-responses-api-proxy

`codex-responses-api-proxy` is a standalone HTTP proxy for OpenAI/Codex-style backend traffic. It forwards requests upstream, streams responses back to the client, and reuses the same usage-accounting tables and usage log files that Codex uses internally.

This makes it a drop-in replacement for tools that normally talk directly to a ChatGPT/OpenAI backend URL. Point those tools at the proxy instead, and the proxy will forward traffic to the real backend while recording usage locally.

## What It Does

- Reads the upstream API key from `stdin`.
- Optionally reads upstream auth from `auth.json` via `--auth-file` instead of stdin, and uses that same file as the accounting identity source.
- Listens on `127.0.0.1` on the requested port, or an ephemeral port if `--port` is omitted.
- Forwards arbitrary HTTP requests to the configured upstream URL.
- Streams SSE responses and observes backend usage events.
- Updates the same `account_usage` SQLite tables and writes the same `usage/*.log` files that Codex already uses.
- Optionally writes request/response exchange dumps for debugging.
- Optionally exposes `GET /shutdown` for environments that cannot send `SIGTERM`.
- Derives the account identity from `--auth-file` when set, otherwise from the incoming client request, so no account-id or account-display CLI flags are needed.

## Usage

Start the proxy with the upstream key on stdin:

```shell
printenv OPENAI_API_KEY | env -u OPENAI_API_KEY codex-responses-api-proxy \
  --http-shutdown \
  --server-info /var/tmp/codex-proxy/server-info.json
```

If you want the proxy to use the auth from `~/.codex/auth.json` instead of stdin, pass `--auth-file`:

```shell
codex-responses-api-proxy \
  --auth-file ~/.codex/auth.json \
  --http-shutdown
```

Point the client application at the proxy instead of the backend directly. For example, if the app normally talks to `https://chatgpt.com/backend-api`, point it at the proxy host and preserve the same path shape:

```shell
PROXY_PORT=$(jq -r .port /var/tmp/codex-proxy/server-info.json)
PROXY_BASE_URL="http://127.0.0.1:${PROXY_PORT}"

# Example Codex override
codex exec \
  -c "model_providers.openai-proxy={ name = 'OpenAI Proxy', base_url = '${PROXY_BASE_URL}/backend-api', wire_api='responses' }" \
  -c model_provider="openai-proxy" \
  'Your prompt here'
```

If the client already speaks the OpenAI Responses API directly, you can point it at the proxy root and let the proxy forward the request to the real upstream:

```shell
PROXY_PORT=$(jq -r .port /var/tmp/codex-proxy/server-info.json)
export OPENAI_BASE_URL="http://127.0.0.1:${PROXY_PORT}"
```

If you want Codex to use the proxy from `~/.codex/config.toml`, point the relevant base URL at the proxy instead of the public backend:

```toml
chatgpt_base_url = "http://127.0.0.1:43128/backend-api"

[model_providers.openai]
base_url = "http://127.0.0.1:43128/v1"
wire_api = "responses"
```

Account identity is derived from the auth file when `--auth-file` is set; otherwise it falls back to the client request in this order:

1. `account_id` / account email / token fingerprint from the selected `auth.json`.
2. `chatgpt-account-id` or `ChatGPT-Account-ID` request header.
3. A stable `auth:<fingerprint>` derived from the client request's `Authorization` header.
4. The fixed fallback bucket `proxy`.

That means `--auth-file` keeps accounting and upstream auth aligned on the same stored account, while existing clients without `--auth-file` can still use request headers or a stable local usage bucket.

For streamed Responses requests, the proxy records usage from `response.completed` when the backend includes it. If the completed event omits usage, the proxy follows up with `GET /responses/{response_id}` and backfills the usage block before writing to the local accounting store.

When finished:

```shell
curl --fail --silent --show-error "http://127.0.0.1:${PROXY_PORT}/shutdown"
```

## CLI

```shell
codex-responses-api-proxy \
  [--port <PORT>] \
  [--server-info <FILE>] \
  [--http-shutdown] \
  [--upstream-url <URL>] \
  [--dump-dir <DIR>] \
  [--sqlite-home <DIR>] \
  [--auth-file <FILE>] \
  [--provider-id <ID>]
```

- `--upstream-url <URL>`: Base upstream URL to forward to. Defaults to `https://api.openai.com`.
- `--sqlite-home <DIR>`: Overrides the local Codex SQLite home. If omitted, the proxy uses `CODEX_SQLITE_HOME` when set, otherwise `~/.codex`.
- `--auth-file <FILE>`: Uses the auth token stored in the specified `auth.json` file for upstream requests instead of reading stdin, and derives the accounting identity from that same file when possible.
- `--provider-id <ID>`: Provider key written into the usage tables. Defaults to `proxy`.
- `--dump-dir <DIR>`: Writes `<prefix>-request.json` and `<prefix>-response.json` files for each proxied exchange.

When `--auth-file` is set, `kill -HUP <pid>` makes the proxy reload that file and swap in the updated upstream bearer token and accounting identity without restarting the process. If no `--auth-file` was configured, the proxy still handles `SIGHUP` but only logs that there is nothing to reload.

After `rebuild_codex.sh`, the proxy binary is installed as `~/.cargo/bin/codex-responses-api-proxy` and versioned copies are kept alongside it, just like `codex`.

## Compatibility Notes

- The proxy currently expects the backend to speak the same Responses/SSE shape that Codex already handles.
- Requests that are not SSE-based are still forwarded, but usage accounting only happens when the backend returns recognized usage metadata.
- `CONNECT` tunneling is not implemented in this binary. Use direct base URL redirection for HTTPS-backed clients, or add MITM handling in a separate transport layer if you need true transparent TLS interception.

## Logging And Accounting

Usage accounting is handled by `codex-state::AccountUsageStore`, so the proxy writes to the same:

- `account_usage` SQLite tables, under the derived client account key
- `usage/usage-*.log` files
- threshold logs for the 100% and 101% limit crossings

Because the proxy reuses the same storage layer, it honors the same `CODEX_HOME`, `CODEX_SQLITE_HOME`, and `CODEX_USAGE_LOG_DIR` conventions that Codex already uses.
