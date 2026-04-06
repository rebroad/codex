const crypto = require("node:crypto");
const path = require("node:path");
const { bytesOf, readJsonlLines, safeJsonParse } = require("./jsonl");

const UUID_RE =
  /\b[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}\b/gi;
const ISO_RE =
  /\b\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(?:\.\d+)?Z\b/g;
const BIG_NUMBER_RE = /\b\d{3,}\b/g;
const HOME_PATH_RE = /\/home\/[^\s"']+/g;

function sha256(input) {
  return crypto.createHash("sha256").update(input).digest("hex");
}

function clip(text, maxChars = 160) {
  if (typeof text !== "string") {
    return "";
  }
  const normalized = text.replace(/\s+/g, " ").trim();
  if (normalized.length <= maxChars) {
    return normalized;
  }
  return `${normalized.slice(0, maxChars)}…`;
}

function extractMessageText(content) {
  if (!Array.isArray(content)) {
    return "";
  }
  return content
    .map((part) => (typeof part?.text === "string" ? part.text : ""))
    .join("")
    .trim();
}

function extractPrimaryString(entry) {
  const payload = entry?.payload;
  if (!payload || typeof payload !== "object") {
    return null;
  }
  if (typeof payload.output === "string") {
    return payload.output;
  }
  if (typeof payload.arguments === "string") {
    return payload.arguments;
  }
  if (typeof payload.message === "string") {
    return payload.message;
  }
  if (typeof payload.text === "string") {
    return payload.text;
  }
  if (Array.isArray(payload.content)) {
    const text = extractMessageText(payload.content);
    if (text.length > 0) {
      return text;
    }
  }
  return null;
}

function normalizeForNearDupes(value) {
  return value
    .replace(UUID_RE, "<uuid>")
    .replace(ISO_RE, "<ts>")
    .replace(HOME_PATH_RE, "<path>")
    .replace(BIG_NUMBER_RE, "<n>")
    .replace(/\s+/g, " ")
    .trim();
}

function pushTopN(items, candidate, limit) {
  items.push(candidate);
  items.sort((a, b) => b.bytes - a.bytes);
  if (items.length > limit) {
    items.length = limit;
  }
}

function mapGetOrSet(map, key, factory) {
  const existing = map.get(key);
  if (existing !== undefined) {
    return existing;
  }
  const created = factory();
  map.set(key, created);
  return created;
}

function summarizeCallArguments(argsText) {
  const preview = clip(argsText, 140);
  return {
    preview,
    hash: sha256(argsText),
  };
}

async function analyzeRollout(filePath, options = {}) {
  const topN = options.topN ?? 20;
  const largeThresholdBytes = options.largeThresholdBytes ?? 256 * 1024;
  const exactDuplicateMinBytes = options.exactDuplicateMinBytes ?? 32 * 1024;
  const nearDuplicateMinBytes = options.nearDuplicateMinBytes ?? 128 * 1024;

  const largestEntries = [];
  const parseErrors = [];
  const exactLargeStringHashes = new Map();
  const nearDuplicateBuckets = new Map();
  const callById = new Map();
  const repetitiveCalls = new Map();
  const pruneCandidates = [];

  let totalLines = 0;
  let totalBytes = 0;

  for await (const lineRec of readJsonlLines(filePath)) {
    totalLines += 1;
    totalBytes += lineRec.lineBytes;

    const parsed = safeJsonParse(lineRec.line);
    if (!parsed.ok) {
      parseErrors.push({
        line: lineRec.lineNo,
        error: parsed.error,
      });
      continue;
    }

    const record = parsed.value;
    const recordType = typeof record?.type === "string" ? record.type : "(unknown)";
    const payloadType =
      typeof record?.payload?.type === "string" ? record.payload.type : "(none)";
    const lineBytes = lineRec.lineBytes;

    pushTopN(
      largestEntries,
      {
        line: lineRec.lineNo,
        bytes: lineBytes,
        type: recordType,
        payloadType,
        preview: clip(lineRec.line, 180),
      },
      topN,
    );

    if (recordType === "response_item" && payloadType === "function_call") {
      const callId = record?.payload?.call_id;
      const name = record?.payload?.name;
      const args = record?.payload?.arguments;
      if (
        typeof callId === "string" &&
        typeof name === "string" &&
        typeof args === "string"
      ) {
        callById.set(callId, {
          name,
          ...summarizeCallArguments(args),
        });
      }
    }

    const primaryString = extractPrimaryString(record);
    if (typeof primaryString !== "string") {
      continue;
    }

    const stringBytes = bytesOf(primaryString);
    const isHugeString = stringBytes >= largeThresholdBytes;
    const stringPreview = clip(primaryString, 180);

    if (stringBytes >= exactDuplicateMinBytes) {
      const hash = sha256(primaryString);
      const dup = mapGetOrSet(exactLargeStringHashes, hash, () => ({
        hash,
        bytes: stringBytes,
        count: 0,
        lines: [],
        type: recordType,
        payloadType,
        preview: stringPreview,
      }));
      dup.count += 1;
      if (dup.lines.length < 20) {
        dup.lines.push(lineRec.lineNo);
      }
    }

    if (stringBytes >= nearDuplicateMinBytes) {
      const normalized = normalizeForNearDupes(primaryString);
      const normalizedKey = sha256(normalized.slice(0, 2000));
      const nearDup = mapGetOrSet(nearDuplicateBuckets, normalizedKey, () => ({
        normalizedKey,
        normalizedPreview: clip(normalized, 200),
        count: 0,
        totalBytes: 0,
        lines: [],
      }));
      nearDup.count += 1;
      nearDup.totalBytes += stringBytes;
      if (nearDup.lines.length < 20) {
        nearDup.lines.push(lineRec.lineNo);
      }
    }

    if (recordType === "response_item" && payloadType === "function_call_output") {
      const callId = record?.payload?.call_id;
      const callMeta =
        typeof callId === "string" ? callById.get(callId) : undefined;
      const callSig = callMeta
        ? `${callMeta.name}:${callMeta.hash}`
        : `unknown:${typeof callId === "string" ? callId : "none"}`;

      const agg = mapGetOrSet(repetitiveCalls, callSig, () => ({
        callSig,
        callName: callMeta?.name ?? "(unknown)",
        argsPreview: callMeta?.preview ?? "",
        count: 0,
        totalOutputBytes: 0,
        maxOutputBytes: 0,
        lines: [],
      }));
      agg.count += 1;
      agg.totalOutputBytes += stringBytes;
      if (stringBytes > agg.maxOutputBytes) {
        agg.maxOutputBytes = stringBytes;
      }
      if (agg.lines.length < 20) {
        agg.lines.push(lineRec.lineNo);
      }

      if (isHugeString) {
        pruneCandidates.push({
          line: lineRec.lineNo,
          bytes: stringBytes,
          reason: "huge function_call_output payload",
          callName: agg.callName,
          argsPreview: agg.argsPreview,
          preview: stringPreview,
        });
      }
    }
  }

  const exactDuplicateLargePayloads = [...exactLargeStringHashes.values()]
    .filter((item) => item.count > 1)
    .sort(
      (a, b) =>
        b.bytes * b.count - a.bytes * a.count || b.count - a.count || b.bytes - a.bytes,
    )
    .slice(0, topN);

  const nearDuplicateLargePayloads = [...nearDuplicateBuckets.values()]
    .filter((item) => item.count > 1)
    .sort((a, b) => b.totalBytes - a.totalBytes || b.count - a.count)
    .slice(0, topN);

  const repetitiveToolCalls = [...repetitiveCalls.values()]
    .filter((item) => item.count > 1)
    .sort((a, b) => b.totalOutputBytes - a.totalOutputBytes || b.count - a.count)
    .slice(0, topN);

  pruneCandidates.sort((a, b) => b.bytes - a.bytes || a.line - b.line);

  return {
    file: path.resolve(filePath),
    totals: {
      lines: totalLines,
      bytes: totalBytes,
    },
    parseErrors,
    largestEntries,
    exactDuplicateLargePayloads,
    nearDuplicateLargePayloads,
    repetitiveToolCalls,
    pruneCandidates: pruneCandidates.slice(0, topN * 2),
    thresholds: {
      largeThresholdBytes,
      exactDuplicateMinBytes,
      nearDuplicateMinBytes,
    },
  };
}

module.exports = {
  analyzeRollout,
};

