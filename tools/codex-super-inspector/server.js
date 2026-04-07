#!/usr/bin/env node
const fs = require("node:fs/promises");
const path = require("node:path");
const http = require("node:http");
const { URL } = require("node:url");

const DEFAULT_PORT = 8789;
const PUBLIC_DIR = path.join(__dirname, "public");
const LOG_PATTERN = /^codex-super\.(?<pid>\d+)\.(?<dir>c2s|s2c)\.log$/;

function json(res, statusCode, value) {
  res.writeHead(statusCode, {
    "content-type": "application/json; charset=utf-8",
    "cache-control": "no-store",
  });
  res.end(JSON.stringify(value, null, 2));
}

function text(res, statusCode, value) {
  res.writeHead(statusCode, {
    "content-type": "text/plain; charset=utf-8",
    "cache-control": "no-store",
  });
  res.end(value);
}

function contentType(filePath) {
  if (filePath.endsWith(".html")) return "text/html; charset=utf-8";
  if (filePath.endsWith(".css")) return "text/css; charset=utf-8";
  if (filePath.endsWith(".js")) return "application/javascript; charset=utf-8";
  return "application/octet-stream";
}

async function serveStatic(res, requestedPath) {
  const relative = requestedPath === "/" ? "/index.html" : requestedPath;
  const safeRelative = path.normalize(relative).replace(/^(\.\.(\/|\\|$))+/, "");
  const filePath = path.join(PUBLIC_DIR, safeRelative);
  if (!filePath.startsWith(PUBLIC_DIR)) {
    text(res, 400, "invalid path");
    return;
  }
  try {
    const data = await fs.readFile(filePath);
    res.writeHead(200, {
      "content-type": contentType(filePath),
      "cache-control": "no-store",
    });
    res.end(data);
  } catch (error) {
    if (error && error.code === "ENOENT") {
      text(res, 404, "not found");
      return;
    }
    text(res, 500, "failed to load static file");
  }
}

function parsePort(argv) {
  const idx = argv.indexOf("--port");
  if (idx === -1) return DEFAULT_PORT;
  const value = Number.parseInt(argv[idx + 1] ?? "", 10);
  if (!Number.isFinite(value) || value <= 0) {
    throw new Error(`invalid --port value: ${argv[idx + 1]}`);
  }
  return value;
}

function parseDefaultTarget(argv) {
  const idx = argv.indexOf("--target");
  if (idx === -1) return null;
  return path.resolve(argv[idx + 1] ?? "");
}

function parseLine(filePath, direction, rawLine, lineNo) {
  const line = rawLine.trim();
  if (!line) return null;
  const match = line.match(/^\[(?<ts>[^\]]+)\]\s*(?<body>.*)$/);
  const timestamp = match?.groups?.ts ?? null;
  const body = (match?.groups?.body ?? line).trim();
  let parsed = null;
  if (body.startsWith("{") || body.startsWith("[")) {
    try {
      parsed = JSON.parse(body);
    } catch (_error) {
      parsed = null;
    }
  }
  const timestampMs = timestamp ? Date.parse(timestamp) : Number.NaN;
  return {
    file: filePath,
    direction,
    lineNo,
    timestamp,
    timestampMs: Number.isFinite(timestampMs) ? timestampMs : null,
    body,
    parsed,
    summary: summarize(parsed, body),
  };
}

function summarize(parsed, body) {
  if (!parsed || typeof parsed !== "object") {
    return body.slice(0, 180);
  }
  if (typeof parsed.method === "string") {
    const id = parsed.id ? ` id=${parsed.id}` : "";
    return `${parsed.method}${id}`;
  }
  if (typeof parsed.type === "string") {
    const id = parsed.id ? ` id=${parsed.id}` : "";
    return `${parsed.type}${id}`;
  }
  return JSON.stringify(parsed).slice(0, 180);
}

async function parseLogFile(filePath, direction) {
  const textData = await fs.readFile(filePath, "utf8");
  const lines = textData.split(/\r?\n/);
  const events = [];
  for (let i = 0; i < lines.length; i += 1) {
    const event = parseLine(filePath, direction, lines[i], i + 1);
    if (event) events.push(event);
  }
  return events;
}

async function discoverSessions(targetPath) {
  const stat = await fs.stat(targetPath);
  if (stat.isFile()) {
    return sessionsFromSingleFile(targetPath);
  }
  if (!stat.isDirectory()) {
    throw new Error("target must be a codex-super log file or a directory");
  }
  const entries = await fs.readdir(targetPath, { withFileTypes: true });
  const groups = new Map();
  for (const entry of entries) {
    if (!entry.isFile()) continue;
    const match = entry.name.match(LOG_PATTERN);
    if (!match || !match.groups) continue;
    const key = match.groups.pid;
    const direction = match.groups.dir;
    const filePath = path.join(targetPath, entry.name);
    const existing = groups.get(key) ?? {
      key,
      pid: key,
      c2s: null,
      s2c: null,
      mtimeMs: 0,
    };
    if (direction === "c2s") existing.c2s = filePath;
    if (direction === "s2c") existing.s2c = filePath;
    try {
      const fileStat = await fs.stat(filePath);
      existing.mtimeMs = Math.max(existing.mtimeMs, fileStat.mtimeMs);
    } catch (_error) {
      // Ignore races.
    }
    groups.set(key, existing);
  }
  return [...groups.values()].sort((a, b) => b.mtimeMs - a.mtimeMs);
}

function sessionsFromSingleFile(filePath) {
  const base = path.basename(filePath);
  const match = base.match(LOG_PATTERN);
  if (!match || !match.groups) {
    throw new Error(
      `target file must match codex-super.<PID>.{c2s,s2c}.log, got: ${base}`,
    );
  }
  const key = match.groups.pid;
  const dirName = path.dirname(filePath);
  const c2sPath = path.join(dirName, `codex-super.${key}.c2s.log`);
  const s2cPath = path.join(dirName, `codex-super.${key}.s2c.log`);
  const session = {
    key,
    pid: key,
    c2s: null,
    s2c: null,
    mtimeMs: 0,
  };
  if (match.groups.dir === "c2s") session.c2s = filePath;
  if (match.groups.dir === "s2c") session.s2c = filePath;
  session.c2s = session.c2s || c2sPath;
  session.s2c = session.s2c || s2cPath;
  return [session];
}

function buildMergedEvents(c2sEvents, s2cEvents) {
  const all = [...c2sEvents, ...s2cEvents];
  all.sort((a, b) => {
    const aHas = typeof a.timestampMs === "number";
    const bHas = typeof b.timestampMs === "number";
    if (aHas && bHas && a.timestampMs !== b.timestampMs) {
      return a.timestampMs - b.timestampMs;
    }
    if (aHas !== bHas) return aHas ? -1 : 1;
    if (a.direction !== b.direction) return a.direction.localeCompare(b.direction);
    return a.lineNo - b.lineNo;
  });
  return all;
}

async function latestTarget() {
  const sessions = await discoverSessions("/tmp").catch(() => []);
  if (!sessions.length) return null;
  const latest = sessions[0];
  return latest.c2s || latest.s2c || null;
}

function canonicalEvent(event, index) {
  return {
    index,
    direction: event.direction,
    timestamp: event.timestamp,
    lineNo: event.lineNo,
    summary: event.summary,
    body: event.body,
    parsed: event.parsed,
    file: event.file,
  };
}

async function handleSessions(res, targetPath) {
  const sessions = await discoverSessions(targetPath);
  json(res, 200, {
    target: targetPath,
    sessions: sessions.map((session) => ({
      key: session.key,
      pid: session.pid,
      hasC2s: Boolean(session.c2s),
      hasS2c: Boolean(session.s2c),
      c2s: session.c2s,
      s2c: session.s2c,
      mtimeMs: session.mtimeMs,
    })),
  });
}

async function handleSession(res, targetPath, sessionKey) {
  const sessions = await discoverSessions(targetPath);
  const session = sessions.find((entry) => entry.key === sessionKey);
  if (!session) {
    json(res, 404, { error: `session not found: ${sessionKey}` });
    return;
  }

  let c2sEvents = [];
  let s2cEvents = [];
  if (session.c2s) {
    try {
      c2sEvents = await parseLogFile(session.c2s, "c2s");
    } catch (_error) {
      c2sEvents = [];
    }
  }
  if (session.s2c) {
    try {
      s2cEvents = await parseLogFile(session.s2c, "s2c");
    } catch (_error) {
      s2cEvents = [];
    }
  }

  const merged = buildMergedEvents(c2sEvents, s2cEvents);
  json(res, 200, {
    target: targetPath,
    session: {
      key: session.key,
      pid: session.pid,
      c2s: session.c2s,
      s2c: session.s2c,
    },
    totals: {
      c2s: c2sEvents.length,
      s2c: s2cEvents.length,
      merged: merged.length,
    },
    events: {
      c2s: c2sEvents.map(canonicalEvent),
      s2c: s2cEvents.map(canonicalEvent),
      merged: merged.map(canonicalEvent),
    },
  });
}

async function createServer(defaultTarget) {
  return http.createServer(async (req, res) => {
    const reqUrl = new URL(req.url || "/", "http://127.0.0.1");
    if (reqUrl.pathname === "/api/health") {
      json(res, 200, { ok: true });
      return;
    }

    if (reqUrl.pathname === "/api/sessions") {
      let target = reqUrl.searchParams.get("target");
      if (!target) target = defaultTarget;
      if (!target) target = await latestTarget();
      if (!target) {
        json(res, 404, { error: "no codex-super captures found" });
        return;
      }
      try {
        await handleSessions(res, path.resolve(target));
      } catch (error) {
        json(res, 400, { error: error instanceof Error ? error.message : String(error) });
      }
      return;
    }

    if (reqUrl.pathname === "/api/session") {
      let target = reqUrl.searchParams.get("target");
      if (!target) target = defaultTarget;
      if (!target) target = await latestTarget();
      const sessionKey = reqUrl.searchParams.get("session");
      if (!target || !sessionKey) {
        json(res, 400, { error: "target and session are required" });
        return;
      }
      try {
        await handleSession(res, path.resolve(target), sessionKey);
      } catch (error) {
        json(res, 400, { error: error instanceof Error ? error.message : String(error) });
      }
      return;
    }

    await serveStatic(res, reqUrl.pathname);
  });
}

async function main() {
  const port = parsePort(process.argv);
  const defaultTarget = parseDefaultTarget(process.argv);
  const server = await createServer(defaultTarget);
  await new Promise((resolve, reject) => {
    server.on("error", reject);
    server.listen(port, "127.0.0.1", resolve);
  });

  const targetMessage = defaultTarget ? ` default target=${defaultTarget}` : "";
  console.log(`codex-super-inspector listening on http://127.0.0.1:${port}${targetMessage}`);
}

main().catch((error) => {
  console.error(error instanceof Error ? error.message : String(error));
  process.exit(1);
});
