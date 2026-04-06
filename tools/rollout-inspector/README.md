# Rollout Inspector

Local tools to inspect Codex rollout `.jsonl` files in two ways:

- Browser renderer for thread-like viewing (`tools/rollout-inspector/server.js`)
- CLI payload analyzer for large/redundant blobs (`tools/rollout-inspector/analyze-rollout.js`)

## 1) Start the web inspector

```bash
node tools/rollout-inspector/server.js --port 8787 --codex-home ~/.codex
```

Open:

- `http://127.0.0.1:8787`

API endpoints:

- `GET /api/files?root=/path/to/.codex` lists recent rollout files under `sessions/` and `archived_sessions/`
- `GET /api/thread?file=/abs/path/rollout.jsonl` returns a simplified thread view
- `GET /api/analyze?file=/abs/path/rollout.jsonl&top=20&largeKb=256` returns large/redundant payload analysis

## 2) Run CLI analysis

```bash
node tools/rollout-inspector/analyze-rollout.js ~/.codex/sessions/2026/03/08/rollout-....jsonl
```

Options:

- `--top <n>`: top list size
- `--large-kb <n>`: threshold used for "huge payload" candidates
- `--json`: emit structured JSON report

## What “redundant” means here

The analyzer reports:

- Exact duplicate large payloads (same SHA-256 hash, repeated lines)
- Near-duplicate large payloads (normalized text fingerprints)
- Repeated tool call signatures with large outputs (`function_call` args + output size)
- Direct prune candidates (very large `function_call_output` lines)

This is designed to identify likely pruning opportunities without mutating the source files.

