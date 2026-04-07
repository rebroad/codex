# Codex Super Inspector

Local browser viewer for `codex-super.PID.{c2s,s2c}.log`.

It supports:

- Session discovery by PID
- `c2s` only view
- `s2c` only view
- Merged chronological view (`c2s` + `s2c` interleaved by timestamp)
- Text filtering and per-event raw JSON expansion

## One-shot command

Open latest `codex-super.*` capture in browser:

```bash
node tools/codex-super-inspector/view-codex-super.js
```

Open a specific log file:

```bash
node tools/codex-super-inspector/view-codex-super.js /tmp/codex-super.2770233.c2s.log
```

Open a directory and browse all sessions:

```bash
node tools/codex-super-inspector/view-codex-super.js /tmp
```

Options:

- `--session <pid>` preselect session key
- `--port <n>` set local server port
- `--no-open` print URL only

## Manual server mode

```bash
node tools/codex-super-inspector/server.js --port 8789 --target /tmp
```

Open:

- `http://127.0.0.1:8789`

API:

- `GET /api/sessions?target=/tmp`
- `GET /api/session?target=/tmp&session=2770233`
