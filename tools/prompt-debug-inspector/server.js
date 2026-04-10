#!/usr/bin/env node
const fs = require("node:fs/promises");
const path = require("node:path");
const http = require("node:http");
const { URL } = require("node:url");

const DEFAULT_PORT = 8788;
const PUBLIC_DIR = path.join(__dirname, "public");

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

function parseDefaultTarget(argv) {
  const idx = argv.indexOf("--target");
  if (idx === -1) {
    return null;
  }
  return path.resolve(argv[idx + 1] ?? "");
}

async function latestCaptureDir() {
  let entries;
  try {
    entries = await fs.readdir("/tmp", { withFileTypes: true });
  } catch (_error) {
    return null;
  }

  const candidates = [];
  for (const entry of entries) {
    if (!entry.isDirectory()) {
      continue;
    }
    if (!entry.name.startsWith("codex-prompt-debug.")) {
      continue;
    }
    const fullPath = path.join("/tmp", entry.name);
    try {
      const stat = await fs.stat(fullPath);
      candidates.push({ path: fullPath, mtimeMs: stat.mtimeMs });
    } catch (_error) {
      // Ignore races while scanning /tmp.
    }
  }

  candidates.sort((a, b) => b.mtimeMs - a.mtimeMs);
  return candidates[0]?.path ?? null;
}

function parseMaybeJson(raw) {
  if (typeof raw !== "string") {
    return raw;
  }
  try {
    return JSON.parse(raw);
  } catch (_error) {
    return raw;
  }
}

function isObject(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function previewValue(value, maxChars = 800) {
  let text;
  if (typeof value === "string") {
    text = value;
  } else {
    try {
      text = JSON.stringify(value);
    } catch (_error) {
      text = String(value);
    }
  }
  if (text.length <= maxChars) {
    return text;
  }
  return `${text.slice(0, maxChars)}…`;
}

function summarizeStreamRecord(record) {
  const transport = record.data?.transport || "";
  const payload = parseMaybeJson(record.data?.payload);
  const payloadType = Array.isArray(payload) ? "array" : typeof payload;
  const eventType = isObject(payload) && typeof payload.type === "string" ? payload.type : "";
  let summary = "";

  if (isObject(payload)) {
    if (typeof payload.delta === "string") {
      summary = payload.delta;
    } else if (typeof payload.text === "string") {
      summary = payload.text;
    } else if (isObject(payload.item) && typeof payload.item.type === "string") {
      const phase = typeof payload.item.phase === "string" ? ` phase=${payload.item.phase}` : "";
      const status =
        typeof payload.item.status === "string" ? ` status=${payload.item.status}` : "";
      summary = `item=${payload.item.type}${phase}${status}`;
    } else if (isObject(payload.response)) {
      const status =
        typeof payload.response.status === "string" ? ` status=${payload.response.status}` : "";
      summary = `${payload.type || "response"}${status}`;
    } else {
      summary = previewValue(payload, 240);
    }
  } else {
    summary = previewValue(payload, 240);
  }

  return {
    line: record.line,
    transport,
    payloadType,
    eventType,
    summary,
    payloadPreview: previewValue(payload, 1600),
  };
}

function extractTextFromMessageContent(content) {
  if (!Array.isArray(content)) {
    return "";
  }
  return content
    .map((item) => {
      if (!item || typeof item !== "object") {
        return "";
      }
      if (typeof item.text === "string") {
        return item.text;
      }
      if (typeof item.input_text === "string") {
        return item.input_text;
      }
      if (typeof item.output_text === "string") {
        return item.output_text;
      }
      return "";
    })
    .filter(Boolean)
    .join("\n");
}

function summarizeInputItem(item, index) {
  if (!item || typeof item !== "object") {
    return {
      index,
      type: "(unknown)",
      summary: String(item),
      raw: item,
    };
  }

  if (item.type === "message") {
    const text = extractTextFromMessageContent(item.content);
    return {
      index,
      type: item.type,
      role: item.role || "(unknown)",
      summary: text || "(no text content)",
      raw: item,
    };
  }

  if (item.type === "function_call_output") {
    const outputText =
      typeof item.output === "string" ? item.output : JSON.stringify(item.output ?? "");
    return {
      index,
      type: item.type,
      callId: item.call_id || "",
      summary: outputText,
      raw: item,
    };
  }

  return {
    index,
    type: item.type || "(unknown)",
    summary: JSON.stringify(item),
    raw: item,
  };
}

function summarizeTool(tool, index) {
  if (!tool || typeof tool !== "object") {
    return { index, kind: "(unknown)", name: "", raw: tool };
  }
  const kind = tool.type || "(unknown)";
  return {
    index,
    kind,
    name: tool.name || tool.function?.name || "",
    raw: tool,
  };
}

function promptShape(payload) {
  if (!payload || typeof payload !== "object") {
    return false;
  }
  return payload.type === "response.create" || Array.isArray(payload.input);
}

function stableJson(value) {
  if (value === null || typeof value !== "object") {
    return JSON.stringify(value);
  }
  if (Array.isArray(value)) {
    return `[${value.map(stableJson).join(",")}]`;
  }
  const keys = Object.keys(value).sort();
  return `{${keys.map((k) => `${JSON.stringify(k)}:${stableJson(value[k])}`).join(",")}}`;
}

async function parseNdjsonFile(filePath) {
  const data = await fs.readFile(filePath, "utf8");
  const lines = data.split(/\r?\n/);
  const records = [];
  const parseErrors = [];
  for (let i = 0; i < lines.length; i += 1) {
    const line = lines[i];
    if (!line.trim()) {
      continue;
    }
    try {
      records.push({
        line: i + 1,
        data: JSON.parse(line),
      });
    } catch (error) {
      parseErrors.push({
        line: i + 1,
        error: error instanceof Error ? error.message : String(error),
      });
    }
  }
  return { records, parseErrors };
}

async function resolveInputFiles(targetPath) {
  const stat = await fs.stat(targetPath);
  if (stat.isDirectory()) {
    const entries = await fs.readdir(targetPath, { withFileTypes: true });
    return entries
      .filter((entry) => entry.isFile() && entry.name.endsWith("_input.ndjson"))
      .map((entry) => path.join(targetPath, entry.name))
      .sort((a, b) => {
        const aNum = Number.parseInt(path.basename(a, "_input.ndjson"), 10);
        const bNum = Number.parseInt(path.basename(b, "_input.ndjson"), 10);
        if (Number.isFinite(aNum) && Number.isFinite(bNum)) {
          return aNum - bNum;
        }
        return a.localeCompare(b);
      });
  }

  if (stat.isFile()) {
    return [targetPath];
  }

  return [];
}

function queryIdFromPath(filePath) {
  const base = path.basename(filePath);
  const match = /^(.+)_input\.ndjson$/.exec(base);
  return match ? match[1] : base;
}

async function responseIdFromOutputFile(outputFilePath) {
  try {
    const { records } = await parseNdjsonFile(outputFilePath);
    let responseId = null;
    for (const record of records) {
      if (!record.data || typeof record.data !== "object") {
        continue;
      }
      const payload = parseMaybeJson(record.data.payload);
      if (!payload || typeof payload !== "object") {
        continue;
      }
      const eventType = payload.type;
      if (eventType !== "response.created" && eventType !== "response.completed") {
        continue;
      }
      const id = payload.response?.id || payload.id;
      if (typeof id === "string" && id) {
        responseId = id;
      }
    }
    return responseId;
  } catch (_error) {
    return null;
  }
}

async function buildResponseIdIndex(targetPath) {
  const files = await resolveInputFiles(targetPath);
  const responseIdToQueryId = {};
  for (const filePath of files) {
    const queryId = queryIdFromPath(filePath);
    const outputFilePath = filePath.replace(/_input\.ndjson$/, "_output.ndjson");
    const responseId = await responseIdFromOutputFile(outputFilePath);
    if (responseId) {
      responseIdToQueryId[responseId] = queryId;
    }
  }
  return responseIdToQueryId;
}

async function buildQueriesIndex(targetPath) {
  const files = await resolveInputFiles(targetPath);
  const responseIdToQueryId = await buildResponseIdIndex(targetPath);
  const queryToResponseId = Object.fromEntries(
    Object.entries(responseIdToQueryId).map(([responseId, queryId]) => [queryId, responseId]),
  );
  const queries = [];

  for (const filePath of files) {
    const queryId = queryIdFromPath(filePath);
    const { records, parseErrors } = await parseNdjsonFile(filePath);
    let latestPrompt = null;
    for (const record of records) {
      const payload = parseMaybeJson(record.data.payload);
      if (promptShape(payload)) {
        latestPrompt = payload;
      }
    }

    queries.push({
      queryId,
      file: filePath,
      records: records.length,
      parseErrors: parseErrors.length,
      model: latestPrompt?.model || "(unknown)",
      inputItems: Array.isArray(latestPrompt?.input) ? latestPrompt.input.length : 0,
      tools: Array.isArray(latestPrompt?.tools) ? latestPrompt.tools.length : 0,
      instructionsChars: (latestPrompt?.instructions || "").length,
      previousResponseId: latestPrompt?.previous_response_id || "",
      responseId: queryToResponseId[queryId] || "",
    });
  }

  return { queries, responseIdToQueryId };
}

async function buildPromptView(targetPath, queryId) {
  const files = await resolveInputFiles(targetPath);
  const responseIdToQueryId = await buildResponseIdIndex(targetPath);
  const selected = files.find((filePath) => queryIdFromPath(filePath) === queryId);
  if (!selected) {
    const err = new Error(`query not found: ${queryId}`);
    err.statusCode = 404;
    throw err;
  }

  const { records, parseErrors } = await parseNdjsonFile(selected);
  const prompts = [];
  const seen = new Map();

  for (const record of records) {
    const payload = parseMaybeJson(record.data.payload);
    if (!promptShape(payload)) {
      continue;
    }

    const key = stableJson(payload);
    const existing = seen.get(key);
    if (existing) {
      existing.duplicateCount += 1;
      existing.lines.push(record.line);
      continue;
    }

    const inputItems = (payload.input || []).map(summarizeInputItem);
    const tools = (payload.tools || []).map(summarizeTool);
    const promptRecord = {
      line: record.line,
      lines: [record.line],
      duplicateCount: 1,
      transport: record.data.transport || "",
      model: payload.model || "",
      type: payload.type || "",
      previousResponseId: payload.previous_response_id || "",
      previousResponseQueryId: responseIdToQueryId[payload.previous_response_id || ""] || null,
      instructions: payload.instructions || "",
      inputItems,
      tools,
      rawPayload: payload,
    };
    prompts.push(promptRecord);
    seen.set(key, promptRecord);
  }

  const outputFile = selected.replace(/_input\.ndjson$/, "_output.ndjson");
  const reasoningFile = selected.replace(/_input\.ndjson$/, "_reasoning.ndjson");
  const output = await buildStreamView(outputFile);
  const reasoning = await buildStreamView(reasoningFile);

  return {
    queryId,
    file: selected,
    totals: {
      lines: records.length,
      parseErrors: parseErrors.length,
      promptVariants: prompts.length,
      promptOccurrences: prompts.reduce((acc, item) => acc + item.duplicateCount, 0),
    },
    responseIdToQueryId,
    parseErrors,
    prompts,
    output,
    reasoning,
  };
}

async function buildStreamView(filePath) {
  try {
    const { records, parseErrors } = await parseNdjsonFile(filePath);
    return {
      file: filePath,
      totals: {
        lines: records.length,
        parseErrors: parseErrors.length,
      },
      parseErrors,
      records: records.map(summarizeStreamRecord),
    };
  } catch (error) {
    if (error && error.code === "ENOENT") {
      return {
        file: filePath,
        missing: true,
        totals: { lines: 0, parseErrors: 0 },
        parseErrors: [],
        records: [],
      };
    }
    throw error;
  }
}

async function handleApi(res, urlObj, defaultTarget) {
  if (urlObj.pathname === "/api/health") {
    json(res, 200, { ok: true });
    return;
  }

  let target = urlObj.searchParams.get("target");
  if (!target) {
    target = defaultTarget ?? (await latestCaptureDir());
  }
  if (!target) {
    json(res, 400, {
      error: "missing target (and no /tmp/codex-prompt-debug.* directory found)",
    });
    return;
  }

  const resolvedTarget = path.resolve(target);

  if (urlObj.pathname === "/api/queries") {
    try {
      const result = await buildQueriesIndex(resolvedTarget);
      json(res, 200, {
        target: resolvedTarget,
        queries: result.queries,
        responseIdToQueryId: result.responseIdToQueryId,
      });
    } catch (error) {
      json(res, 500, {
        error: error instanceof Error ? error.message : String(error),
      });
    }
    return;
  }

  if (urlObj.pathname === "/api/query") {
    const queryId = urlObj.searchParams.get("queryId");
    if (!queryId) {
      json(res, 400, { error: "missing queryId" });
      return;
    }
    try {
      const view = await buildPromptView(resolvedTarget, queryId);
      json(res, 200, {
        target: resolvedTarget,
        ...view,
      });
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
  const argv = process.argv.slice(2);
  const port = parsePort(argv);
  const defaultTarget = parseDefaultTarget(argv);

  const server = http.createServer(async (req, res) => {
    const urlObj = new URL(req.url || "/", "http://127.0.0.1");

    if (urlObj.pathname.startsWith("/api/")) {
      await handleApi(res, urlObj, defaultTarget);
      return;
    }

    await serveStatic(res, urlObj.pathname);
  });

  server.listen(port, "127.0.0.1", () => {
    console.log(`prompt-debug-inspector listening at http://127.0.0.1:${port}`);
    if (defaultTarget) {
      console.log(`default target: ${defaultTarget}`);
    } else {
      console.log("default target: latest /tmp/codex-prompt-debug.*");
    }
  });
}

main().catch((error) => {
  console.error(error instanceof Error ? error.message : String(error));
  process.exit(1);
});
