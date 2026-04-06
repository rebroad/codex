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
  turn.items.push(item);
}

function summarizeToolOutput(payload, maxToolChars) {
  const output = typeof payload.output === "string" ? payload.output : "";
  const outputBytes = bytesOf(output);
  return {
    type: "toolOutput",
    callId: payload.call_id ?? null,
    outputBytes,
    outputPreview: clip(output, maxToolChars),
  };
}

async function buildThreadView(filePath, options = {}) {
  const includeToolCalls = options.includeToolCalls ?? true;
  const includeReasoning = options.includeReasoning ?? false;
  const includeSystemMessages = options.includeSystemMessages ?? false;
  const maxToolChars = options.maxToolChars ?? 800;

  const state = {
    sessionMeta: null,
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

    if (recordType === "session_meta") {
      state.sessionMeta = payload;
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
            text: typeof payload.message === "string" ? payload.message : "(empty user message)",
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

    if (payload.type === "function_call" || payload.type === "custom_tool_call") {
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
    totals: state.totals,
    parseErrors: state.parseErrors,
    turns: state.turns,
  };
}

module.exports = {
  buildThreadView,
};

