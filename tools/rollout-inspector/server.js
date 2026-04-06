#!/usr/bin/env node
const fs = require("node:fs/promises");
const path = require("node:path");
const http = require("node:http");
const { URL } = require("node:url");
const { analyzeRollout } = require("./lib/analyzer");
const { buildThreadView } = require("./lib/thread_view");

const DEFAULT_PORT = 8787;
const PUBLIC_DIR = path.join(__dirname, "public");

function parseBoolean(value, defaultValue = false) {
  if (value === null) {
    return defaultValue;
  }
  return value === "1" || value === "true" || value === "yes";
}

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
  if (filePath.endsWith(".html")) {
    return "text/html; charset=utf-8";
  }
  if (filePath.endsWith(".css")) {
    return "text/css; charset=utf-8";
  }
  if (filePath.endsWith(".js")) {
    return "application/javascript; charset=utf-8";
  }
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

function requireFileParam(urlObj) {
  const file = urlObj.searchParams.get("file");
  if (!file) {
    const err = new Error("missing required query param: file");
    err.statusCode = 400;
    throw err;
  }
  const resolved = path.resolve(file);
  if (!resolved.endsWith(".jsonl")) {
    const err = new Error("file must point to a .jsonl path");
    err.statusCode = 400;
    throw err;
  }
  return resolved;
}

async function listRolloutFiles(rootPath, limit = 400) {
  const roots = ["sessions", "archived_sessions"].map((segment) =>
    path.join(rootPath, segment),
  );
  const out = [];

  async function visit(dirPath) {
    if (out.length >= limit) {
      return;
    }
    let entries;
    try {
      entries = await fs.readdir(dirPath, { withFileTypes: true });
    } catch (error) {
      if (error && error.code === "ENOENT") {
        return;
      }
      throw error;
    }

    for (const entry of entries) {
      if (out.length >= limit) {
        return;
      }
      const fullPath = path.join(dirPath, entry.name);
      if (entry.isDirectory()) {
        await visit(fullPath);
      } else if (entry.isFile() && entry.name.endsWith(".jsonl")) {
        const stat = await fs.stat(fullPath);
        out.push({
          path: fullPath,
          sizeBytes: stat.size,
          mtimeMs: stat.mtimeMs,
        });
      }
    }
  }

  for (const root of roots) {
    await visit(root);
  }

  out.sort((a, b) => b.mtimeMs - a.mtimeMs);
  return out.slice(0, limit);
}

function parsePort(argv) {
  const idx = argv.indexOf("--port");
  if (idx === -1) {
    return DEFAULT_PORT;
  }
  const value = Number.parseInt(argv[idx + 1] ?? "", 10);
  if (!Number.isFinite(value) || value <= 0) {
    throw new Error(`invalid --port value: ${argv[idx + 1]}`);
  }
  return value;
}

function parseCodexHome(argv) {
  const idx = argv.indexOf("--codex-home");
  if (idx === -1) {
    return path.join(process.env.HOME || "", ".codex");
  }
  return path.resolve(argv[idx + 1] ?? "");
}

async function handleApi(req, res, urlObj, codexHome) {
  if (urlObj.pathname === "/api/health") {
    json(res, 200, { ok: true });
    return;
  }

  if (urlObj.pathname === "/api/files") {
    try {
      const root = urlObj.searchParams.get("root")
        ? path.resolve(urlObj.searchParams.get("root"))
        : codexHome;
      const limit = Number.parseInt(urlObj.searchParams.get("limit") ?? "400", 10);
      const files = await listRolloutFiles(root, Number.isFinite(limit) ? limit : 400);
      json(res, 200, { root, files });
    } catch (error) {
      json(res, 500, {
        error: error instanceof Error ? error.message : String(error),
      });
    }
    return;
  }

  if (urlObj.pathname === "/api/thread") {
    try {
      const filePath = requireFileParam(urlObj);
      const thread = await buildThreadView(filePath, {
        includeToolCalls: parseBoolean(urlObj.searchParams.get("includeTools"), true),
        includeReasoning: parseBoolean(urlObj.searchParams.get("includeReasoning"), false),
        includeSystemMessages: parseBoolean(
          urlObj.searchParams.get("includeSystemMessages"),
          false,
        ),
        maxToolChars:
          Number.parseInt(urlObj.searchParams.get("maxToolChars") ?? "800", 10) || 800,
      });
      json(res, 200, thread);
    } catch (error) {
      const statusCode = error?.statusCode ?? 500;
      json(res, statusCode, {
        error: error instanceof Error ? error.message : String(error),
      });
    }
    return;
  }

  if (urlObj.pathname === "/api/analyze") {
    try {
      const filePath = requireFileParam(urlObj);
      const top = Number.parseInt(urlObj.searchParams.get("top") ?? "20", 10);
      const largeKb = Number.parseInt(urlObj.searchParams.get("largeKb") ?? "256", 10);
      const report = await analyzeRollout(filePath, {
        topN: Number.isFinite(top) && top > 0 ? top : 20,
        largeThresholdBytes:
          Number.isFinite(largeKb) && largeKb > 0 ? largeKb * 1024 : 256 * 1024,
      });
      json(res, 200, report);
    } catch (error) {
      const statusCode = error?.statusCode ?? 500;
      json(res, statusCode, {
        error: error instanceof Error ? error.message : String(error),
      });
    }
    return;
  }

  json(res, 404, { error: "unknown API route" });
}

async function main() {
  const port = parsePort(process.argv.slice(2));
  const codexHome = parseCodexHome(process.argv.slice(2));

  const server = http.createServer(async (req, res) => {
    const urlObj = new URL(req.url || "/", "http://127.0.0.1");

    if (urlObj.pathname.startsWith("/api/")) {
      await handleApi(req, res, urlObj, codexHome);
      return;
    }

    await serveStatic(res, urlObj.pathname);
  });

  server.listen(port, "127.0.0.1", () => {
    console.log(`rollout-inspector listening at http://127.0.0.1:${port}`);
    console.log(`default CODEX_HOME root: ${codexHome}`);
  });
}

main().catch((error) => {
  console.error(error instanceof Error ? error.message : String(error));
  process.exit(1);
});

