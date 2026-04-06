function $(id) {
  return document.getElementById(id);
}

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

function renderThread(thread) {
  const view = $("threadView");
  const meta = $("threadMeta");
  view.innerHTML = "";
  meta.textContent = "";

  if (!thread) {
    view.innerHTML = `<div class="empty">(no data)</div>`;
    return;
  }

  meta.textContent =
    `file=${thread.file}\n` +
    `session=${thread.session.id || "(unknown)"} source=${thread.session.source || "(unknown)"} cwd=${thread.session.cwd || "(none)"}\n` +
    `lines=${thread.totals.lines} bytes=${thread.totals.bytes}`;

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
      const msgNode = document.createElement("div");
      msgNode.className = `msg ${roleClass(item.role || "other")}`;
      if (item.type === "message") {
        msgNode.textContent = item.text || "";
      } else if (item.type === "toolCall") {
        msgNode.innerHTML =
          `<span class="kind">tool call: ${item.name || "(unknown)"}</span>` +
          `${item.argsPreview || "(no args)"}`;
      } else if (item.type === "toolOutput") {
        msgNode.innerHTML =
          `<span class="kind">tool output: ${formatBytes(item.outputBytes || 0)}</span>` +
          `${item.outputPreview || "(no output)"}`;
      } else if (item.type === "reasoning") {
        msgNode.innerHTML =
          `<span class="kind">reasoning</span>` + `${item.text || "(empty reasoning)"}`;
      } else {
        msgNode.textContent = JSON.stringify(item);
      }
      turnNode.appendChild(msgNode);
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
        .map((c) => `<td>${String(c.value(row) ?? "").replaceAll("<", "&lt;")}</td>`)
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
  html += renderTable("Exact Duplicate Large Payloads", report.exactDuplicateLargePayloads, [
    { header: "count", value: (r) => r.count },
    { header: "bytes each", value: (r) => formatBytes(r.bytes) },
    { header: "total", value: (r) => formatBytes(r.bytes * r.count) },
    { header: "lines", value: (r) => (r.lines || []).join(",") },
    { header: "preview", value: (r) => r.preview },
  ]);
  html += renderTable("Near-Duplicate Large Payloads", report.nearDuplicateLargePayloads, [
    { header: "count", value: (r) => r.count },
    { header: "total", value: (r) => formatBytes(r.totalBytes) },
    { header: "lines", value: (r) => (r.lines || []).join(",") },
    { header: "normalized preview", value: (r) => r.normalizedPreview },
  ]);
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
  const query = rootPath ? `?root=${encodeURIComponent(rootPath)}` : "";

  const data = await getJson(`/api/files${query}`);
  for (const item of data.files || []) {
    const li = document.createElement("li");
    li.textContent = `${item.path} (${formatBytes(item.sizeBytes)})`;
    li.addEventListener("click", () => {
      $("filePath").value = item.path;
    });
    filesNode.appendChild(li);
  }
  if (!filesNode.firstChild) {
    filesNode.innerHTML = `<li class="empty">(no rollout files found)</li>`;
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
loadRecentFiles().catch(() => {
  // Ignore at load time; user can retry manually.
});

