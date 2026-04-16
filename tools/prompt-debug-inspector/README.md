# Prompt Debug Inspector

Local browser viewer for backend capture directories (`codex-backend-capture.*`, with legacy `codex-prompt-debug.*` support).

It renders per-query capture data in a human-readable layout:

- Instructions text
- Input items (messages, function call outputs, etc.)
- Tool list
- Raw prompt JSON (for exact inspection)
- Output stream events (`*_output.ndjson`)
- Reasoning stream events (`*_reasoning.ndjson`)

## One-shot command

Open latest capture directory in browser:

```bash
node tools/prompt-debug-inspector/view-prompt-debug.js
```

Open a specific capture directory:

```bash
node tools/prompt-debug-inspector/view-prompt-debug.js /var/tmp/codex-backend-capture.2770233
```

Open a specific query input file:

```bash
node tools/prompt-debug-inspector/view-prompt-debug.js /var/tmp/codex-backend-capture.2770233/22_input.ndjson
```

Options:

- `--query-id <id>` preselect query (when target is a directory)
- `--port <n>` set local server port
- `--no-open` print URL only

## Manual server mode

```bash
node tools/prompt-debug-inspector/server.js --port 8788 --target /var/tmp/codex-backend-capture.2770233
```

Open:

- `http://127.0.0.1:8788`

API:

- `GET /api/queries?target=/var/tmp/codex-backend-capture.2770233`
- `GET /api/query?target=/var/tmp/codex-backend-capture.2770233&queryId=22`
