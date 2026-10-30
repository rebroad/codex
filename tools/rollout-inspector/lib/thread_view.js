const path = require("node:path");
const { readJsonlLines, safeJsonParse, bytesOf } = require("./jsonl");

function extractMessageText(content) {
  if (!Array.isArray(content)) {
    return "";
  }
  return content
    .map((part) => (typeof part?.text === "string" ? part.text : ""))
    .join("")
    .trim();
}

function clip(text, maxChars) {
  if (typeof text !== "string") {
    return "";
  }
  if (text.length <= maxChars) {
    return text;
  }
  return `${text.slice(0, maxChars)}…`;
}

function ensureTurn(state, reason) {
  if (state.currentTurn) {
    return state.currentTurn;
  }
  const turn = {
    id: `implicit-${state.turns.length + 1}`,
    status: reason ?? "implicit",
    items: [],
  };
  state.turns.push(turn);
  state.currentTurn = turn;
  return turn;
}

function pushTurnItem(state, item, reason) {
  const turn = ensureTurn(state, reason);
  turn.items.push({ ...state.recordMeta, ...item });
}

function summarizeToolOutput(payload, maxToolChars) {
  const output = payload.output;
  const outputText = typeof output === "string"
    ? output
    : Array.isArray(output)
      ? output.map((part) => {
          if (typeof part?.text === "string") return part.text;
          return part?.type ? `[${part.type} content]` : "[non-text content]";
        }).join("\n")
      : output == null
        ? ""
        : JSON.stringify(output);
  const outputBytes = bytesOf(outputText);
  return {
    type: "toolOutput",
    callId: payload.call_id ?? null,
    outputBytes,
    outputPreview: outputText
      ? clip(outputText, maxToolChars)
      : Object.hasOwn(payload, "output")
        ? "(empty output)"
        : "(output field missing)",
  };
}

function omissionReason(record) {
  const payload = record?.payload ?? {};
  if (record?.type === "session_meta") return "session metadata shown in header";
  if (record?.type === "world_state" || record?.type === "turn_context") {
    return `${record.type} snapshot not rendered`;
  }
  if (record?.type === "event_msg") {
    if (["task_started", "turn_started", "task_complete", "turn_complete"].includes(payload.type)) {
      return "turn boundary shown by section heading";
    }
    if (payload.type === "item_completed") return "item lifecycle event not rendered";
    if (payload.type === "token_count") return "token/rate telemetry not rendered";
    if (payload.type === "account_updated") return "account metadata event not rendered";
    return `event ${payload.type ?? "(unknown)"} not rendered`;
  }
  if (record?.type === "response_item") {
    if (payload.type === "message") {
      return `${payload.role ?? "other"} message hidden; enable system/developer messages`;
    }
    if (payload.type === "reasoning") return "reasoning hidden; enable include reasoning";
    if (["function_call", "custom_tool_call", "function_call_output", "custom_tool_call_output"].includes(payload.type)) {
      return "tool record hidden; enable include tool calls";
    }
    return `response ${payload.type ?? "(unknown)"} not rendered`;
  }
  return `record ${record?.type ?? "(unknown)"} not rendered`;
}

function addOmissionSummaries(state) {
  const turns = state.turns;
  const visibleOrdinals = new Set(
    turns.flatMap((turn) => turn.items.map((item) => item.ordinal))
      .filter(Number.isInteger),
  );
  const catalog = state.recordCatalog.sort((a, b) => a.ordinal - b.ordinal);
  const entries = catalog
    .filter((entry) => !visibleOrdinals.has(entry.ordinal))
    .map((entry) => ({
      startOrdinal: entry.ordinal,
      endOrdinal: entry.ordinal,
      timestamp: entry.timestamp,
      turnId: entry.turnId,
      reason: entry.reason,
    }));

  for (let i = 1; i < catalog.length; i += 1) {
    const previous = catalog[i - 1];
    const current = catalog[i];
    if (current.ordinal > previous.ordinal + 1) {
      entries.push({
        startOrdinal: previous.ordinal + 1,
        endOrdinal: current.ordinal - 1,
        timestamp: current.timestamp,
        turnId: current.turnId,
        reason: "no records exist for these ordinal numbers",
      });
    }
  }
  entries.sort((a, b) => a.startOrdinal - b.startOrdinal);

  const groups = [];
  for (const entry of entries) {
    const previous = groups.at(-1);
    if (
      previous &&
      previous.endOrdinal + 1 === entry.startOrdinal
    ) {
      previous.endOrdinal = entry.endOrdinal;
      previous.reasons[entry.reason] = (previous.reasons[entry.reason] ?? 0) +
        entry.endOrdinal - entry.startOrdinal + 1;
    } else {
      groups.push({
        ...entry,
        reasons: {
          [entry.reason]: entry.endOrdinal - entry.startOrdinal + 1,
        },
      });
    }
  }

  for (const group of groups) {
    let turn = turns.find((candidate) => candidate.id === group.turnId);
    if (!turn) {
      const following = catalog.find(
        (entry) =>
          entry.ordinal >= group.startOrdinal &&
          turns.some((candidate) => candidate.id === entry.turnId),
      );
      turn = turns.find((candidate) => candidate.id === following?.turnId);
    }
    turn ??= turns.at(-1);
    if (turn) {
      turn.items.push({
        type: "omissionSummary",
        ordinal: group.startOrdinal,
        endOrdinal: group.endOrdinal,
        timestamp: group.timestamp,
        summary: Object.entries(group.reasons)
          .sort((a, b) => b[1] - a[1])
          .slice(0, 4)
          .map(([reason, count]) => `${reason} (${count})`)
          .concat(Object.keys(group.reasons).length > 4
            ? [`${Object.keys(group.reasons).length - 4} other omission reasons`]
            : [])
          .join("; "),
      });
    }
  }

  for (const turn of turns) {
    turn.items.sort((a, b) => (a.ordinal ?? 0) - (b.ordinal ?? 0));
  }
}

async function buildThreadView(filePath, options = {}) {
  const includeToolCalls = options.includeToolCalls ?? true;
  const includeReasoning = options.includeReasoning ?? false;
  const includeSystemMessages = options.includeSystemMessages ?? false;
  const maxToolChars = options.maxToolChars ?? 800;

  const state = {
    sessionMeta: null,
    models: new Set(),
    tokenUsage: null,
    recordMeta: null,
    recordCatalog: [],
    turns: [],
    currentTurn: null,
    totals: {
      lines: 0,
      bytes: 0,
    },
    parseErrors: [],
  };

  for await (const lineRec of readJsonlLines(filePath)) {
    state.totals.lines += 1;
    state.totals.bytes += lineRec.lineBytes;

    const parsed = safeJsonParse(lineRec.line);
    if (!parsed.ok) {
      state.parseErrors.push({
        line: lineRec.lineNo,
        error: parsed.error,
      });
      continue;
    }
    const record = parsed.value;
    const recordType = record?.type;
    const payload = record?.payload ?? {};
    state.recordMeta = {
      ordinal: record?.ordinal ?? null,
      timestamp: record?.timestamp ?? null,
    };
    if (Number.isInteger(record?.ordinal)) {
      state.recordCatalog.push({
        ordinal: record.ordinal,
        timestamp: record.timestamp ?? null,
        turnId: payload.turn_id ?? state.currentTurn?.id ?? null,
        reason: omissionReason(record),
      });
    }

    if (recordType === "session_meta") {
      state.sessionMeta = payload;
      continue;
    }

    if (recordType === "token_usage_record") {
      if (typeof payload.effective_model === "string") {
        state.models.add(payload.effective_model);
      }
      state.tokenUsage = {
        latestResponse: payload.usage ?? null,
        thread: payload.thread_token_usage ?? payload.turn_token_usage ?? null,
      };
      pushTurnItem(state, {
        type: "tokenUsage",
        model: payload.effective_model ?? null,
        usage: payload.usage ?? null,
      }, "token_usage_record");
      continue;
    }

    if (recordType === "event_msg") {
      const evType = payload?.type;
      if (evType === "task_started" || evType === "turn_started") {
        state.currentTurn = {
          id: payload?.turn_id ?? `turn-${state.turns.length + 1}`,
          status: evType,
          items: [],
        };
        state.turns.push(state.currentTurn);
        continue;
      }
      if (evType === "task_complete" || evType === "turn_complete") {
        if (state.currentTurn) {
          state.currentTurn.status = evType;
        }
        state.currentTurn = null;
        continue;
      }
      if (evType === "user_message") {
        pushTurnItem(
          state,
          {
            type: "message",
            role: "user",
            text:
              typeof payload.message === "string"
                ? payload.message
                : "(empty user message)",
          },
          "event",
        );
        continue;
      }
      if (evType === "agent_message") {
        pushTurnItem(
          state,
          {
            type: "message",
            role: "assistant",
            text:
              typeof payload.message === "string"
                ? payload.message
                : "(empty assistant message)",
          },
          "event",
        );
      }
      continue;
    }

    if (recordType !== "response_item") {
      continue;
    }

    if (payload.type === "message") {
      const role = typeof payload.role === "string" ? payload.role : "other";
      if (!includeSystemMessages && role !== "user" && role !== "assistant") {
        continue;
      }
      const text = extractMessageText(payload.content);
      pushTurnItem(
        state,
        {
          type: "message",
          role,
          text: text || `(empty ${role} message)`,
        },
        "response_item",
      );
      continue;
    }

    if (payload.type === "reasoning") {
      if (!includeReasoning) {
        continue;
      }
      pushTurnItem(
        state,
        {
          type: "reasoning",
          role: "reasoning",
          text: clip(JSON.stringify(payload), 1200),
        },
        "reasoning",
      );
      continue;
    }

    if (
      payload.type === "function_call" ||
      payload.type === "custom_tool_call"
    ) {
      if (!includeToolCalls) {
        continue;
      }
      pushTurnItem(
        state,
        {
          type: "toolCall",
          toolType: payload.type,
          callId: payload.call_id ?? null,
          name: payload.name ?? payload.tool_name ?? "(unknown tool)",
          argsPreview: clip(payload.arguments ?? payload.input ?? "", 600),
        },
        "tool",
      );
      continue;
    }

    if (
      payload.type === "function_call_output" ||
      payload.type === "custom_tool_call_output"
    ) {
      if (!includeToolCalls) {
        continue;
      }
      pushTurnItem(state, summarizeToolOutput(payload, maxToolChars), "tool");
    }
  }

  addOmissionSummaries(state);
  const ordinalNumbers = state.recordCatalog
    .map((entry) => entry.ordinal)
    .sort((a, b) => a - b);
  const firstOrdinal = ordinalNumbers.length ? Math.min(...ordinalNumbers) : null;
  const lastOrdinal = ordinalNumbers.length ? Math.max(...ordinalNumbers) : null;
  const ordinalCount = new Set(ordinalNumbers).size;
  const continuous = ordinalNumbers.every((ordinal, index) => ordinal === firstOrdinal + index);

  return {
    file: path.resolve(filePath),
    session: {
      id: state.sessionMeta?.id ?? null,
      cwd: state.sessionMeta?.cwd ?? null,
      source: state.sessionMeta?.source ?? null,
      originator: state.sessionMeta?.originator ?? null,
      timestamp: state.sessionMeta?.timestamp ?? null,
      modelProvider: state.sessionMeta?.model_provider ?? null,
    },
    models: [...state.models],
    tokenUsage: state.tokenUsage,
    ordinalCoverage: {
      first: firstOrdinal,
      last: lastOrdinal,
      count: ordinalCount,
      continuous: firstOrdinal === null || continuous,
    },
    totals: state.totals,
    parseErrors: state.parseErrors,
    turns: state.turns,
  };
}

module.exports = {
  buildThreadView,
};
