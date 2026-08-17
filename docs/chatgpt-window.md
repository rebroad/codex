# ChatGPT Window provider

`chatgpt-window` is an opt-in local Responses provider backed by a signed-in
ChatGPT tab in Chrome. It is supplied by the separate `chatgpt-window`
project; Codex communicates with it through the standard Responses API and does
not modify the normal OpenAI backend.

Install/start the local bridge and adapter once, then check them:

```bash
chatgpt-window --setup
curl --fail http://127.0.0.1:8767/health
```

Add a provider and profile to `~/.codex/config.toml`:

```toml
[model_providers.chatgpt-window]
name = "ChatGPT Window"
base_url = "http://127.0.0.1:8767/v1"
wire_api = "responses"
requires_openai_auth = false
supports_websockets = false

```

Create `$CODEX_HOME/chatgpt-window.config.toml` with:

```toml
model_provider = "chatgpt-window"
model = "chatgpt-window"
```

Select it with `codex -p chatgpt-window`. The adapter maintains a mapping from
Codex thread IDs to ChatGPT conversation IDs and relays text, supported
attachments, reasoning effort, and tool calls. Tool calls are validated by the
adapter but executed by Codex, so Codex approvals, sandboxing, MCP, and policy
remain authoritative.

The same profile also namespaces the local app-server daemon. The standard
server and the ChatGPT Window server can therefore run at the same time without
using a second `CODEX_HOME`:

```bash
codex app-server daemon start
codex -p chatgpt-window app-server daemon start
```

The unprofiled server keeps the existing
`$CODEX_HOME/app-server-control/app-server-control.sock` and daemon state. The
profiled server uses a profile-specific socket and daemon-state directory under
`$CODEX_HOME`. Proxy and TUI auto-connect operations select the same profile
when invoked with `-p chatgpt-window`.

The services must remain available and the Chrome extension must remain
connected. The browser bridge permits one active request per conversation.
