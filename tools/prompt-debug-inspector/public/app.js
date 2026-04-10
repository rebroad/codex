function $(id) {
  return document.getElementById(id);
}

let currentTarget = "";
let currentQueryId = "";

function escapeHtml(value) {
  return String(value ?? "")
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;");
}

async function getJson(path) {
  const res = await fetch(path);
  const data = await res.json();
  if (!res.ok) {
    throw new Error(data.error || `HTTP ${res.status}`);
  }
  return data;
}

function renderQueriesMeta(target, queries) {
  $("queriesMeta").textContent =
    `target=${target}\n` +
    `queries=${queries.length}`;
}

function queryLabel(query) {
  return (
    `#${query.queryId} model=${query.model} ` +
    `input=${query.inputItems} tools=${query.tools} ` +
    `instr=${query.instructionsChars} chars`
  );
}

function renderQueries(target, queries) {
  currentTarget = target;
  const list = $("queries");
  list.innerHTML = "";
  renderQueriesMeta(target, queries);

  if (!Array.isArray(queries) || queries.length === 0) {
    list.innerHTML = `<li class="empty">(no *_input.ndjson files found)</li>`;
    return;
  }

  for (const query of queries) {
    const li = document.createElement("li");
    li.dataset.queryId = query.queryId;
    li.innerHTML =
      `<div>${escapeHtml(queryLabel(query))}</div>` +
      `<div class="meta">${escapeHtml(query.file)}</div>`;
    li.addEventListener("click", async () => {
      for (const node of list.querySelectorAll("li.active")) {
        node.classList.remove("active");
      }
      li.classList.add("active");
      await loadQuery(query.queryId);
    });
    list.appendChild(li);
  }
}

function renderInputItem(item) {
  const cls = item.type === "message" ? item.role || "" : item.type || "";
  const metaParts = [`#${item.index + 1}`, item.type || "(unknown)"];
  if (item.role) {
    metaParts.push(`role=${item.role}`);
  }
  if (item.callId) {
    metaParts.push(`call_id=${item.callId}`);
  }
  return (
    `<div class="input-item ${escapeHtml(cls)}">` +
    `<div class="item-meta">${escapeHtml(metaParts.join(" · "))}</div>` +
    `<pre>${escapeHtml(item.summary || "")}</pre>` +
    `</div>`
  );
}

function renderPrimitive(value) {
  if (typeof value === "string") {
    return JSON.stringify(value);
  }
  if (value === undefined) {
    return "undefined";
  }
  return JSON.stringify(value);
}

function renderJsonNode(key, value, depth) {
  const keyLabel = key === null ? "" : `${key}: `;

  if (Array.isArray(value)) {
    const title = `${keyLabel}[${value.length}]`;
    const children =
      value.length === 0
        ? `<div class="json-leaf">(empty array)</div>`
        : value.map((child, idx) => renderJsonNode(String(idx), child, depth + 1)).join("");
    return (
      `<details class="json-node" ${depth <= 1 ? "open" : ""}>` +
      `<summary>${escapeHtml(title)}</summary>` +
      `<div class="json-children">${children}</div>` +
      `</details>`
    );
  }

  if (value && typeof value === "object") {
    const keys = Object.keys(value);
    const title = `${keyLabel}{${keys.length}}`;
    const children =
      keys.length === 0
        ? `<div class="json-leaf">(empty object)</div>`
        : keys.map((childKey) => renderJsonNode(childKey, value[childKey], depth + 1)).join("");
    return (
      `<details class="json-node" ${depth <= 1 ? "open" : ""}>` +
      `<summary>${escapeHtml(title)}</summary>` +
      `<div class="json-children">${children}</div>` +
      `</details>`
    );
  }

  return `<div class="json-leaf">${escapeHtml(`${keyLabel}${renderPrimitive(value)}`)}</div>`;
}

function renderToolBody(raw) {
  if (!raw || typeof raw !== "object") {
    return `<pre>${escapeHtml(JSON.stringify(raw, null, 2))}</pre>`;
  }
  const keys = Object.keys(raw);
  if (keys.length === 0) {
    return `<div class="json-leaf">(empty object)</div>`;
  }
  return `<div class="json-tree">${keys
    .map((key) => renderJsonNode(key, raw[key], 0))
    .join("")}</div>`;
}

function renderTool(tool) {
  const name = tool.name ? ` ${tool.name}` : "";
  return (
    `<details class="tool-item">` +
    `<summary class="tool-pill">${escapeHtml(tool.kind)}${escapeHtml(name)}</summary>` +
    `<div class="tool-body">${renderToolBody(tool.raw)}</div>` +
    `</details>`
  );
}

function renderPrompt(prompt) {
  const chips = [
    `type=${prompt.type || "(none)"}`,
    `model=${prompt.model || "(none)"}`,
    `transport=${prompt.transport || "(none)"}`,
    `lines=${prompt.lines.join(",")}`,
    `occurrences=${prompt.duplicateCount}`,
  ];
  const previousResponseChip = prompt.previousResponseId
    ? prompt.previousResponseQueryId
      ? `<span class="chip">previous_response_id=<a href="#" class="previous-response-link" data-query-id="${escapeHtml(
          prompt.previousResponseQueryId,
        )}" data-response-id="${escapeHtml(prompt.previousResponseId)}">${escapeHtml(
          prompt.previousResponseId,
        )}</a></span>`
      : `<span class="chip">previous_response_id=${escapeHtml(prompt.previousResponseId)}</span>`
    : "";

  return (
    `<section class="prompt-card">` +
    `<div class="chips">${chips.map((chip) => `<span class="chip">${escapeHtml(chip)}</span>`).join("")}${previousResponseChip}</div>` +
    `<div class="section-title">Instructions (${prompt.instructions.length} chars)</div>` +
    `<pre>${escapeHtml(prompt.instructions || "(none)")}</pre>` +
    `<div class="section-title">Input Items (${prompt.inputItems.length})</div>` +
    (prompt.inputItems.length
      ? prompt.inputItems.map(renderInputItem).join("")
      : `<div class="empty">(none)</div>`) +
    `<div class="section-title">Tools (${prompt.tools.length})</div>` +
    (prompt.tools.length
      ? `<div class="tools-list">${prompt.tools.map(renderTool).join("")}</div>`
      : `<div class="empty">(none)</div>`) +
    `<details>` +
    `<summary>Raw Prompt JSON</summary>` +
    `<pre>${escapeHtml(JSON.stringify(prompt.rawPayload, null, 2))}</pre>` +
    `</details>` +
    `</section>`
  );
}

function renderStreamRecord(record) {
  const chips = [
    `line=${record.line}`,
    `transport=${record.transport || "(none)"}`,
    `event=${record.eventType || "(none)"}`,
    `payload=${record.payloadType || "(none)"}`,
  ];
  return (
    `<details class="stream-record">` +
    `<summary>${chips.map((chip) => `<span class="chip">${escapeHtml(chip)}</span>`).join("")}</summary>` +
    `<pre>${escapeHtml(record.summary || "(no summary)")}</pre>` +
    `<div class="section-title">Payload Preview</div>` +
    `<pre>${escapeHtml(record.payloadPreview || "")}</pre>` +
    `</details>`
  );
}

function renderStreamSection(title, stream) {
  if (!stream) {
    return "";
  }
  const records = Array.isArray(stream.records) ? stream.records : [];
  const missing = Boolean(stream.missing);
  return (
    `<section class="panel stream-panel">` +
    `<h2>${escapeHtml(title)}</h2>` +
    `<div class="meta">file=${escapeHtml(stream.file || "(none)")}\nlines=${escapeHtml(
      String(stream.totals?.lines ?? 0),
    )} parse_errors=${escapeHtml(String(stream.totals?.parseErrors ?? 0))}${missing ? "\n(missing file)" : ""}</div>` +
    (records.length
      ? `<div class="stream-list">${records.map(renderStreamRecord).join("")}</div>`
      : `<div class="empty">(no records)</div>`) +
    `</section>`
  );
}

function renderPromptView(view) {
  currentQueryId = view.queryId;
  $("promptMeta").textContent =
    `query=${view.queryId}\n` +
    `file=${view.file}\n` +
    `lines=${view.totals.lines} prompt_variants=${view.totals.promptVariants} occurrences=${view.totals.promptOccurrences} parse_errors=${view.totals.parseErrors}\n` +
    `output_lines=${view.output?.totals?.lines ?? 0} reasoning_lines=${view.reasoning?.totals?.lines ?? 0}`;

  if (!Array.isArray(view.prompts) || view.prompts.length === 0) {
    $("promptView").innerHTML = `<div class="empty">(no response.create payloads detected)</div>`;
  } else {
    $("promptView").innerHTML = view.prompts.map(renderPrompt).join("");
    wirePreviousResponseLinks($("promptView"));
    wireToolAccordions($("promptView"));
  }

  const streams = $("streamsView");
  streams.innerHTML =
    renderStreamSection("Output Stream", view.output) +
    renderStreamSection("Reasoning Stream", view.reasoning);
}

async function loadQueries() {
  const target = $("targetPath").value.trim();
  const query = target ? `?target=${encodeURIComponent(target)}` : "";
  const data = await getJson(`/api/queries${query}`);
  renderQueries(data.target, data.queries || []);

  const params = new URLSearchParams(window.location.search);
  const requestedQueryId = params.get("queryId");
  const firstQuery = requestedQueryId || data.queries?.[0]?.queryId;
  if (firstQuery) {
    const node = [...$("queries").querySelectorAll("li")].find(
      (li) => li.dataset.queryId === firstQuery,
    );
    if (node) {
      node.classList.add("active");
    }
    await loadQuery(firstQuery);
  }
}

async function loadQuery(queryId) {
  const target = $("targetPath").value.trim();
  const params = new URLSearchParams({ queryId });
  if (target) {
    params.set("target", target);
  }
  const data = await getJson(`/api/query?${params.toString()}`);
  renderPromptView(data);
  setActiveQueryListItem(queryId);
}

function applyUrlState() {
  const params = new URLSearchParams(window.location.search);
  const target = params.get("target");
  if (target) {
    $("targetPath").value = target;
  }
}

function attachHandlers() {
  $("loadQueriesBtn").addEventListener("click", () => {
    loadQueries().catch((err) => alert(err.message));
  });
}

function setActiveQueryListItem(queryId) {
  const list = $("queries");
  for (const node of list.querySelectorAll("li.active")) {
    node.classList.remove("active");
  }
  const targetNode = [...list.querySelectorAll("li")].find(
    (li) => li.dataset.queryId === String(queryId),
  );
  if (targetNode) {
    targetNode.classList.add("active");
  }
}

function wireToolAccordions(root) {
  const toolLists = root.querySelectorAll(".tools-list");
  for (const list of toolLists) {
    const items = [...list.querySelectorAll(":scope > details.tool-item")];
    for (const item of items) {
      item.addEventListener("toggle", () => {
        if (item.open) {
          for (const other of items) {
            if (other !== item) {
              other.open = false;
              other.classList.remove("open");
            }
          }
          item.classList.add("open");
          list.classList.add("has-open");
        } else {
          item.classList.remove("open");
          if (!items.some((candidate) => candidate.open)) {
            list.classList.remove("has-open");
          }
        }
      });
    }
  }
}

function wirePreviousResponseLinks(root) {
  const links = root.querySelectorAll("a.previous-response-link");
  for (const link of links) {
    link.addEventListener("click", async (event) => {
      event.preventDefault();
      const queryId = link.dataset.queryId;
      if (!queryId) {
        return;
      }
      await loadQuery(queryId);
    });
  }
}

attachHandlers();
applyUrlState();
loadQueries().catch((err) => {
  $("queriesMeta").textContent = err.message;
});
