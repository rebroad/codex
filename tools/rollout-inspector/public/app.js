function $(id) {
  return document.getElementById(id);
}

let directoryMode = false;

function formatBytes(bytes) {
  const units = ["B", "KiB", "MiB", "GiB"];
  let value = bytes;
  let idx = 0;
  while (value >= 1024 && idx < units.length - 1) {
    value /= 1024;
    idx += 1;
  }
  return `${value.toFixed(value >= 10 || idx === 0 ? 0 : 1)} ${units[idx]}`;
}

async function getJson(path) {
  const res = await fetch(path);
  const data = await res.json();
  if (!res.ok) {
    throw new Error(data.error || `HTTP ${res.status}`);
  }
  return data;
}

function roleClass(role) {
  if (role === "user") {
    return "role-user";
  }
  if (role === "assistant") {
    return "role-assistant";
  }
  if (role === "reasoning") {
    return "role-reasoning";
  }
  return "role-other";
}

function formatTokenCount(value) {
  return typeof value === "number"
    ? new Intl.NumberFormat().format(value)
    : "unknown";
}

function makeOrdinalToggle(ordinal, timestamp, body, initialText) {
  const entry = document.createElement("div");
  entry.className = "ordinal-entry";
  const toggle = document.createElement("button");
  toggle.type = "button";
  toggle.className = "record-meta ordinal-toggle";
  toggle.textContent = initialText;
  toggle.setAttribute("aria-expanded", "true");
  toggle.addEventListener("click", () => {
    body.hidden = !body.hidden;
    toggle.setAttribute("aria-expanded", String(!body.hidden));
    toggle.textContent = body.hidden
      ? `Ordinal ${ordinal} · ${timestamp || "timestamp unknown"} · Expand`
      : `Ordinal ${ordinal} · ${timestamp || "timestamp unknown"} · Collapse`;
  });
  entry.append(toggle, body);
  return entry;
}

function renderItemBody(item) {
  const body = document.createElement("div");
  body.className = "msg-body";
  if (item.type === "message") {
    body.textContent = item.text || "";
  } else if (item.type === "toolCall") {
    const kind = document.createElement("span");
    kind.className = "kind";
    kind.textContent = `tool call: ${item.name || "(unknown tool)"}`;
    body.append(kind, document.createTextNode(item.argsPreview || "(no args)"));
  } else if (item.type === "toolOutput") {
    const kind = document.createElement("span");
    kind.className = "kind";
    kind.textContent = `tool output: ${formatBytes(item.outputBytes || 0)}`;
    body.append(kind, document.createTextNode(item.outputPreview));
  } else if (item.type === "reasoning") {
    const kind = document.createElement("span");
    kind.className = "kind";
    kind.textContent = "reasoning";
    body.append(kind, document.createTextNode(item.text || "(empty reasoning)"));
  } else if (item.type === "tokenUsage") {
    const usage = item.usage || {};
    body.classList.add("token-usage");
    body.textContent =
      `${item.model || "unknown model"} · ` +
      `${formatTokenCount(usage.total_tokens)} total · ` +
      `${formatTokenCount(usage.input_tokens)} input ` +
      `(${formatTokenCount(usage.cached_input_tokens)} cached, ` +
      `${formatTokenCount(usage.cache_write_input_tokens)} cache write) · ` +
      `${formatTokenCount(usage.output_tokens)} output ` +
      `(${formatTokenCount(usage.reasoning_output_tokens)} reasoning)`;
  } else {
    body.textContent = JSON.stringify(item);
  }
  return body;
}

function renderOrdinalItem(item) {
  const message = document.createElement("div");
  message.className = `msg ${roleClass(item.role || "other")}`;
  const body = renderItemBody(item);
  if (item.type === "tokenUsage") body.classList.add("token-usage");
  if (item.ordinal !== null || item.timestamp) {
    const range = item.endOrdinal > item.ordinal
      ? `${item.ordinal}–${item.endOrdinal}`
      : String(item.ordinal ?? "?");
    const toggle = makeOrdinalToggle(
      range,
      item.timestamp,
      body,
      `Ordinal ${range} · ${item.timestamp || "timestamp unknown"} · Collapse`,
    );
    message.appendChild(toggle);
  } else {
    message.appendChild(body);
  }
  return message;
}

function renderRawRecord(record) {
  const payload = record.payload || {};
  const type = payload.type ? `${record.type}/${payload.type}` : record.type;
  const body = document.createElement("pre");
  body.className = "raw-record";
  body.textContent = JSON.stringify(payload, null, 2);
  const message = document.createElement("div");
  message.className = "msg role-other expanded-record";
  const toggle = makeOrdinalToggle(
    record.ordinal,
    record.timestamp,
    body,
    `Ordinal ${record.ordinal} · ${record.timestamp || "timestamp unknown"} · ${type} · Collapse`,
  );
  message.appendChild(toggle);
  return message;
}

function renderOmissionSummary(item, filePath) {
  const entry = document.createElement("div");
  entry.className = "omission-entry";
  const records = document.createElement("div");
  records.className = "omission-records";
  records.hidden = true;
  const range = item.endOrdinal > item.ordinal
    ? `${item.ordinal}–${item.endOrdinal}`
    : String(item.ordinal);
  const label = `Not rendered · ordinals ${range}: ${item.summary}`;
  const toggle = document.createElement("button");
  toggle.type = "button";
  toggle.className = "record-meta ordinal-toggle";
  toggle.textContent = `${label} · Expand`;
  toggle.setAttribute("aria-expanded", "false");
  let loaded = false;
  toggle.addEventListener("click", async () => {
    if (!loaded && records.hidden) {
      toggle.disabled = true;
      toggle.textContent = `${label} · Loading`;
      try {
        const params = new URLSearchParams({
          file: filePath,
          from: String(item.ordinal),
          to: String(item.endOrdinal),
        });
        const result = await getJson(`/api/records?${params.toString()}`);
        if (result.records.length === 0) {
          const empty = document.createElement("div");
          empty.className = "record-meta";
          empty.textContent = "No JSONL records exist for this ordinal range.";
          records.appendChild(empty);
        } else {
          for (const record of result.records) records.appendChild(renderRawRecord(record));
        }
        loaded = true;
      } catch (error) {
        const failure = document.createElement("div");
        failure.className = "record-meta load-error";
        failure.textContent = error instanceof Error ? error.message : String(error);
        records.replaceChildren(failure);
      } finally {
        toggle.disabled = false;
      }
    }
    records.hidden = !records.hidden;
    const expanded = !records.hidden;
    toggle.setAttribute("aria-expanded", String(expanded));
    toggle.textContent = `${label} · ${expanded ? "Collapse" : "Expand"}`;
  });
  entry.append(toggle, records);
  return entry;
}

function renderThread(thread) {
  const view = $("threadView");
  const meta = $("threadMeta");
  const ordinalSummary = $("ordinalSummary");
  view.innerHTML = "";
  meta.textContent = "";
  ordinalSummary.textContent = "";

  if (!thread) {
    view.innerHTML = `<div class="empty">(no data)</div>`;
    return;
  }

  meta.textContent =
    `file=${thread.file}\n` +
    `session=${thread.session.id || "(unknown)"} source=${thread.session.source || "(unknown)"} cwd=${thread.session.cwd || "(none)"}\n` +
    `lines=${thread.totals.lines} bytes=${thread.totals.bytes}`;
  if (thread.ordinalCoverage?.first !== null && thread.ordinalCoverage?.last !== null) {
    const coverage = thread.ordinalCoverage;
    ordinalSummary.textContent = coverage.continuous
      ? `Ordinal coverage ${coverage.first}–${coverage.last}: no ordinal numbers are absent; records not rendered are summarized inline.`
      : `Ordinal coverage ${coverage.first}–${coverage.last}: some ordinal numbers have no record; see inline summaries.`;
  }

  if (!Array.isArray(thread.turns) || thread.turns.length === 0) {
    view.innerHTML = `<div class="empty">(no turns/messages detected)</div>`;
    return;
  }

  for (let i = 0; i < thread.turns.length; i += 1) {
    const turn = thread.turns[i];
    const turnNode = document.createElement("section");
    turnNode.className = "turn";
    turnNode.innerHTML = `<div class="turn-title">Turn ${i + 1} · ${turn.status || "unknown"}</div>`;

    for (const item of turn.items || []) {
      if (item.type === "omissionSummary") {
        turnNode.appendChild(renderOmissionSummary(item, thread.file));
        continue;
      }
      turnNode.appendChild(renderOrdinalItem(item));
    }

    view.appendChild(turnNode);
  }
}

function renderTable(title, rows, columns) {
  if (!rows || rows.length === 0) {
    return `<h3>${title}</h3><div class="empty">(none)</div>`;
  }
  const thead = columns.map((c) => `<th>${c.header}</th>`).join("");
  const body = rows
    .map((row) => {
      const cells = columns
        .map(
          (c) =>
            `<td>${String(c.value(row) ?? "").replaceAll("<", "&lt;")}</td>`,
        )
        .join("");
      return `<tr>${cells}</tr>`;
    })
    .join("");
  return `<h3>${title}</h3><table class="analysis-table"><thead><tr>${thead}</tr></thead><tbody>${body}</tbody></table>`;
}

function renderAnalysis(report) {
  const meta = $("analysisMeta");
  const view = $("analysis");

  if (!report) {
    meta.textContent = "";
    view.innerHTML = `<div class="empty">(no analysis yet)</div>`;
    return;
  }

  meta.textContent = `file=${report.file}\nlines=${report.totals.lines} bytes=${report.totals.bytes} (${formatBytes(report.totals.bytes)})`;

  let html = "";
  html += renderTable("Largest Entries", report.largestEntries, [
    { header: "line", value: (r) => r.line },
    { header: "size", value: (r) => formatBytes(r.bytes) },
    { header: "kind", value: (r) => `${r.type}/${r.payloadType}` },
    { header: "preview", value: (r) => r.preview },
  ]);
  html += renderTable(
    "Exact Duplicate Large Payloads",
    report.exactDuplicateLargePayloads,
    [
      { header: "count", value: (r) => r.count },
      { header: "bytes each", value: (r) => formatBytes(r.bytes) },
      { header: "total", value: (r) => formatBytes(r.bytes * r.count) },
      { header: "lines", value: (r) => (r.lines || []).join(",") },
      { header: "preview", value: (r) => r.preview },
    ],
  );
  html += renderTable(
    "Near-Duplicate Large Payloads",
    report.nearDuplicateLargePayloads,
    [
      { header: "count", value: (r) => r.count },
      { header: "total", value: (r) => formatBytes(r.totalBytes) },
      { header: "lines", value: (r) => (r.lines || []).join(",") },
      { header: "normalized preview", value: (r) => r.normalizedPreview },
    ],
  );
  html += renderTable("Repeated Tool Calls", report.repetitiveToolCalls, [
    { header: "tool", value: (r) => r.callName },
    { header: "count", value: (r) => r.count },
    { header: "total output", value: (r) => formatBytes(r.totalOutputBytes) },
    { header: "max output", value: (r) => formatBytes(r.maxOutputBytes) },
    { header: "lines", value: (r) => (r.lines || []).join(",") },
    { header: "args", value: (r) => r.argsPreview || "" },
  ]);
  html += renderTable("Prune Candidates", report.pruneCandidates, [
    { header: "line", value: (r) => r.line },
    { header: "size", value: (r) => formatBytes(r.bytes) },
    { header: "reason", value: (r) => r.reason },
    { header: "tool", value: (r) => r.callName || "" },
    { header: "preview", value: (r) => r.preview || "" },
  ]);

  view.innerHTML = html;
}

async function loadRecentFiles() {
  const filesNode = $("files");
  filesNode.innerHTML = "";
  const rootPath = $("rootPath").value.trim();
  const params = new URLSearchParams();
  if (rootPath) params.set("root", rootPath);
  if (directoryMode) params.set("directory", "1");
  const query = params.size > 0 ? `?${params.toString()}` : "";

  const data = await getJson(`/api/files${query}`);
  $("fileListTitle").textContent = directoryMode
    ? `Rollouts in ${data.root}`
    : "Recent Files";
  for (const item of data.files || []) {
    const li = document.createElement("li");
    if (directoryMode) {
      const details = document.createElement("details");
      const summary = document.createElement("summary");
      const firstMessage = item.summary?.firstMessage || "(no user message found)";
      summary.textContent = `${new Date(item.mtimeMs).toLocaleString()} · ${firstMessage}`;
      const info = document.createElement("div");
      info.className = "rollout-summary";
      info.textContent = [
        `session=${item.summary?.sessionId || "(unknown)"}`,
        `source=${item.summary?.source || "(unknown)"}`,
        `cwd=${item.summary?.cwd || "(unknown)"}`,
        `size=${formatBytes(item.sizeBytes)}`,
        item.path,
      ].join("\n");
      const openButton = document.createElement("button");
      openButton.textContent = "Load full thread";
      openButton.addEventListener("click", async () => {
        $("filePath").value = item.path;
        await loadThread();
        $("threadView").scrollIntoView({ behavior: "smooth", block: "start" });
      });
      details.append(summary, info, openButton);
      li.appendChild(details);
    } else {
      li.textContent = `${item.path} (${formatBytes(item.sizeBytes)})`;
      li.addEventListener("click", () => {
        $("filePath").value = item.path;
      });
    }
    filesNode.appendChild(li);
  }
  if (!filesNode.firstChild) {
    filesNode.innerHTML = `<li class="empty">(no rollout files found)</li>`;
  }
}

function applyUrlState() {
  const params = new URLSearchParams(window.location.search);
  const file = params.get("file");
  const root = params.get("root");
  directoryMode = params.get("directory") === "1";
  const includeTools = params.get("includeTools");
  const includeReasoning = params.get("includeReasoning");
  const includeSystemMessages = params.get("includeSystemMessages");

  if (file) {
    $("filePath").value = file;
  }
  if (root) {
    $("rootPath").value = root;
  }
  if (includeTools !== null) {
    $("includeTools").checked = includeTools !== "0";
  }
  if (includeReasoning !== null) {
    $("includeReasoning").checked = includeReasoning === "1";
  }
  if (includeSystemMessages !== null) {
    $("includeSystemMessages").checked = includeSystemMessages === "1";
  }
}

async function loadThread() {
  const filePath = $("filePath").value.trim();
  if (!filePath) {
    throw new Error("set rollout file path first");
  }

  const includeTools = $("includeTools").checked ? "1" : "0";
  const includeReasoning = $("includeReasoning").checked ? "1" : "0";
  const includeSystemMessages = $("includeSystemMessages").checked ? "1" : "0";

  const query = new URLSearchParams({
    file: filePath,
    includeTools,
    includeReasoning,
    includeSystemMessages,
  });
  const thread = await getJson(`/api/thread?${query.toString()}`);
  renderThread(thread);
}

async function runAnalysis() {
  const filePath = $("filePath").value.trim();
  if (!filePath) {
    throw new Error("set rollout file path first");
  }
  const query = new URLSearchParams({
    file: filePath,
    top: "20",
    largeKb: "256",
  });
  const report = await getJson(`/api/analyze?${query.toString()}`);
  renderAnalysis(report);
}

function attachHandlers() {
  $("refreshFilesBtn").addEventListener("click", () => {
    loadRecentFiles().catch((err) => alert(err.message));
  });
  $("loadThreadBtn").addEventListener("click", () => {
    loadThread().catch((err) => alert(err.message));
  });
  $("analyzeBtn").addEventListener("click", () => {
    runAnalysis().catch((err) => alert(err.message));
  });
}

attachHandlers();
applyUrlState();

async function bootstrap() {
  await loadRecentFiles().catch(() => {
    // Ignore at load time; user can retry manually.
  });

  const params = new URLSearchParams(window.location.search);
  const autoload = params.get("autoload") === "1";
  const analyze = params.get("analyze") !== "0";
  const hasFile = Boolean($("filePath").value.trim());
  if (!autoload || !hasFile) {
    return;
  }

  await loadThread();
  if (analyze) {
    await runAnalysis();
  }
}

bootstrap().catch((err) => {
  alert(err.message);
});
