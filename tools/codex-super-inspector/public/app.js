function $(id) {
  return document.getElementById(id);
}

const PAGE_LIMIT = 250;
const MODES = ["merged", "c2s", "s2c"];

let currentTarget = "";
let currentSession = "";
let currentData = null;
let loadedEvents = { merged: [], c2s: [], s2c: [] };
let modeState = {
  merged: { offset: 0, total: 0, hasMore: false, loading: false },
  c2s: { offset: 0, total: 0, hasMore: false, loading: false },
  s2c: { offset: 0, total: 0, hasMore: false, loading: false },
};
const replayRequestIds = new Set();

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

function sessionLabel(session) {
  const parts = [`pid=${session.pid}`];
  if (session.hasC2s) parts.push("c2s");
  if (session.hasS2c) parts.push("s2c");
  return parts.join(" · ");
}

function renderSessions(data) {
  currentTarget = data.target;
  const list = $("sessions");
  list.innerHTML = "";
  $("sessionsMeta").textContent = `target=${data.target}\nsessions=${data.sessions.length}`;

  if (!data.sessions.length) {
    list.innerHTML = `<li class="empty">(no codex-super.* logs found)</li>`;
    return;
  }

  for (const session of data.sessions) {
    const li = document.createElement("li");
    li.dataset.session = session.key;
    li.innerHTML =
      `<div>${escapeHtml(sessionLabel(session))}</div>` +
      `<div class="meta">key=${escapeHtml(session.key)}</div>`;
    li.addEventListener("click", async () => {
      for (const node of list.querySelectorAll("li.active")) node.classList.remove("active");
      li.classList.add("active");
      await loadSession(session.key);
    });
    list.appendChild(li);
  }
}

function formatBody(event) {
  if (event.parsed) {
    return JSON.stringify(event.parsed, null, 2);
  }
  return event.body || "";
}

function escapeForHtmlAttr(value) {
  return escapeHtml(value).replaceAll('"', "&quot;");
}

function isSupervisorInjected(event) {
  const meta = event?.parsed?._supervisor;
  return (
    event?.direction === "c2s" &&
    meta &&
    typeof meta === "object" &&
    meta.injectedBy === "supervisor"
  );
}

function eventRequestId(event) {
  const parsed = event?.parsed;
  if (!parsed || typeof parsed !== "object" || !Object.prototype.hasOwnProperty.call(parsed, "id")) {
    return null;
  }
  const id = parsed.id;
  if (typeof id === "string" || typeof id === "number") return id;
  return null;
}

function isSupervisorReplayRequestId(requestId) {
  return typeof requestId === "string" && requestId.startsWith("supervisor/replay/");
}

function isSupervisorSuppressed(event) {
  if (event?.direction !== "s2c") return false;
  const parsed = event?.parsed;
  if (!parsed || typeof parsed !== "object") return false;
  if (!Object.prototype.hasOwnProperty.call(parsed, "result") && !Object.prototype.hasOwnProperty.call(parsed, "error")) {
    return false;
  }
  const requestId = eventRequestId(event);
  if (requestId === null) return false;
  return isSupervisorReplayRequestId(requestId) || replayRequestIds.has(requestId);
}

function renderCollapsedResultDataBody(event) {
  const parsed = event?.parsed;
  if (!parsed || typeof parsed !== "object") return null;
  const responseId = eventRequestId(event);
  if (responseId === null) return null;
  const result = parsed.result;
  if (!result || typeof result !== "object" || !Array.isArray(result.data)) return null;
  if (
    result.data.some(
      (item) =>
        !item ||
        typeof item !== "object" ||
        !Object.prototype.hasOwnProperty.call(item, "name") ||
        !Object.prototype.hasOwnProperty.call(item, "path") ||
        !Object.prototype.hasOwnProperty.call(item, "cwd") ||
        !Object.prototype.hasOwnProperty.call(item, "cliVersion"),
    )
  ) {
    return null;
  }

  const rows = result.data
    .map((item, idx) => {
      if (!item || typeof item !== "object") return "";

      const itemId = item.id ?? `item-${idx + 1}`;
      const name = item.name;
      const path = item.path;
      const cwd = item.cwd;
      const cliVersion = item.cliVersion;
      const full = JSON.stringify(item, null, 2);

      return (
        `<article class="result-item">` +
        `<div class="result-item-id"><code>${escapeHtml(itemId)}</code></div>` +
        `<div class="result-item-meta">` +
        `<div><span class="label">name</span><span title="${escapeForHtmlAttr(String(name))}">${escapeHtml(name)}</span></div>` +
        `<div><span class="label">path</span><span title="${escapeForHtmlAttr(String(path))}">${escapeHtml(path)}</span></div>` +
        `<div><span class="label">cwd</span><span title="${escapeForHtmlAttr(String(cwd))}">${escapeHtml(cwd)}</span></div>` +
        `<div><span class="label">cliVersion</span><span>${escapeHtml(cliVersion)}</span></div>` +
        `</div>` +
        `<details class="result-item-full">` +
        `<summary>Full data</summary>` +
        `<pre>${escapeHtml(full)}</pre>` +
        `</details>` +
        `</article>`
      );
    })
    .filter(Boolean)
    .join("");

  return (
    `<section class="collapsed-result-data">` +
    `<div class="result-response-id">id: <code>${escapeHtml(String(responseId))}</code></div>` +
    `<div class="result-response-count">results: ${result.data.length}</div>` +
    `<div class="result-items">${rows || '<div class="empty">(no object items)</div>'}</div>` +
    `</section>`
  );
}

function renderCollapsedResultThreadBody(event) {
  const parsed = event?.parsed;
  if (!parsed || typeof parsed !== "object") return null;
  const responseId = eventRequestId(event);
  if (responseId === null) return null;
  const result = parsed.result;
  if (!result || typeof result !== "object") return null;
  const thread = result.thread;
  if (!thread || typeof thread !== "object") return null;
  if (
    !Object.prototype.hasOwnProperty.call(thread, "name") ||
    !Object.prototype.hasOwnProperty.call(thread, "path") ||
    !Object.prototype.hasOwnProperty.call(thread, "cwd") ||
    !Object.prototype.hasOwnProperty.call(thread, "cliVersion")
  ) {
    return null;
  }

  const full = JSON.stringify(thread, null, 2);
  return (
    `<section class="collapsed-result-data">` +
    `<div class="result-response-id">id: <code>${escapeHtml(String(responseId))}</code></div>` +
    `<div class="result-items">` +
    `<article class="result-item">` +
    `<div class="result-item-id"><code>${escapeHtml(String(thread.id ?? "(no-thread-id)"))}</code></div>` +
    `<div class="result-item-meta">` +
    `<div><span class="label">name</span><span title="${escapeForHtmlAttr(String(thread.name))}">${escapeHtml(thread.name)}</span></div>` +
    `<div><span class="label">path</span><span title="${escapeForHtmlAttr(String(thread.path))}">${escapeHtml(thread.path)}</span></div>` +
    `<div><span class="label">cwd</span><span title="${escapeForHtmlAttr(String(thread.cwd))}">${escapeHtml(thread.cwd)}</span></div>` +
    `<div><span class="label">cliVersion</span><span>${escapeHtml(thread.cliVersion)}</span></div>` +
    `</div>` +
    `<details class="result-item-full">` +
    `<summary>Full thread</summary>` +
    `<pre>${escapeHtml(full)}</pre>` +
    `</details>` +
    `</article>` +
    `</div>` +
    `</section>`
  );
}

function renderEvent(event, idx) {
  const supervisorInjected = isSupervisorInjected(event);
  const supervisorSuppressed = isSupervisorSuppressed(event);
  const collapsedResultBody =
    renderCollapsedResultDataBody(event) || renderCollapsedResultThreadBody(event);
  const chips = [
    `<span class="chip">#${idx + 1}</span>`,
    `<span class="chip">${escapeHtml(event.direction)}</span>`,
    supervisorInjected ? `<span class="chip chip-supervisor">supervisor</span>` : "",
    supervisorSuppressed ? `<span class="chip chip-suppressed">suppressed</span>` : "",
    event.timestamp ? `<span class="chip">${escapeHtml(event.timestamp)}</span>` : "",
    `<span class="chip">line=${event.lineNo}</span>`,
    `<span class="chip">${escapeHtml(event.summary || "")}</span>`,
  ].join("");

  const classes = ["event-card", escapeHtml(event.direction)];
  if (supervisorInjected) classes.push("supervisor-injected");
  if (supervisorSuppressed) classes.push("supervisor-suppressed");

  return (
    `<article class="${classes.join(" ")}">` +
    `<div class="event-head">${chips}</div>` +
    (collapsedResultBody || `<pre>${escapeHtml(formatBody(event))}</pre>`) +
    `<details><summary>Raw Event</summary><pre>${escapeHtml(JSON.stringify(event, null, 2))}</pre></details>` +
    `</article>`
  );
}

function selectedMode() {
  return $("modeSelect").value;
}

function selectedFilter() {
  return $("filterInput").value.trim().toLowerCase();
}

function filteredEvents(events) {
  const query = selectedFilter();
  if (!query) return events;
  return events.filter((event) => {
    const haystack =
      `${event.summary || ""}\n${event.body || ""}\n${event.timestamp || ""}\n${event.direction}`.toLowerCase();
    return haystack.includes(query);
  });
}

function updateLoadMoreButton() {
  const mode = selectedMode();
  const state = modeState[mode];
  const button = $("loadMoreBtn");
  if (!button) return;
  button.disabled = !currentData || state.loading || !state.hasMore;
  button.textContent = state.loading ? "Loading…" : "Load More";
}

function renderEvents() {
  if (!currentData) {
    $("events").innerHTML = `<div class="empty">(select a session)</div>`;
    $("eventsMeta").textContent = "";
    updateLoadMoreButton();
    return;
  }

  const mode = selectedMode();
  const all = loadedEvents[mode] || [];
  const events = filteredEvents(all);
  const state = modeState[mode];

  $("eventsMeta").textContent =
    `session=${currentData.session.key}\n` +
    `mode=${mode}\n` +
    `shown=${events.length} loaded=${all.length} total=${state.total}\n` +
    `c2s=${currentData.totals.c2s} s2c=${currentData.totals.s2c} merged=${currentData.totals.merged}`;

  $("events").innerHTML = events.length
    ? events.map(renderEvent).join("")
    : `<div class="empty">(no events match current filter)</div>`;

  updateLoadMoreButton();
}

function getActiveTarget() {
  return $("targetPath").value.trim() || currentTarget;
}

function primeReplayRequestIdsFromEvents(events) {
  for (const event of events) {
    if (isSupervisorInjected(event)) {
      const id = eventRequestId(event);
      if (id !== null) replayRequestIds.add(id);
    }
  }
}

async function loadMoreEvents(mode = selectedMode()) {
  if (!currentData) return;
  const state = modeState[mode];
  if (state.loading || !state.hasMore) return;

  state.loading = true;
  updateLoadMoreButton();
  try {
    const params = new URLSearchParams({
      session: currentSession,
      mode,
      offset: String(state.offset),
      limit: String(PAGE_LIMIT),
    });
    const target = getActiveTarget();
    if (target) params.set("target", target);
    const page = await getJson(`/api/events?${params.toString()}`);

    loadedEvents[mode].push(...page.events);
    state.offset = page.nextOffset;
    state.total = page.total;
    state.hasMore = page.hasMore;

    if (mode === "c2s" || mode === "merged") {
      primeReplayRequestIdsFromEvents(page.events);
    }
  } finally {
    state.loading = false;
    renderEvents();
  }
}

async function ensureModeInitialized(mode = selectedMode()) {
  if (!currentData) return;
  if (loadedEvents[mode].length > 0 || !modeState[mode].hasMore) {
    renderEvents();
    return;
  }
  await loadMoreEvents(mode);
}

function resetSessionState(sessionMeta) {
  loadedEvents = { merged: [], c2s: [], s2c: [] };
  replayRequestIds.clear();
  modeState = {
    merged: {
      offset: 0,
      total: sessionMeta.totals.merged,
      hasMore: sessionMeta.totals.merged > 0,
      loading: false,
    },
    c2s: {
      offset: 0,
      total: sessionMeta.totals.c2s,
      hasMore: sessionMeta.totals.c2s > 0,
      loading: false,
    },
    s2c: {
      offset: 0,
      total: sessionMeta.totals.s2c,
      hasMore: sessionMeta.totals.s2c > 0,
      loading: false,
    },
  };
}

async function loadSessions() {
  const target = $("targetPath").value.trim();
  const query = target ? `?target=${encodeURIComponent(target)}` : "";
  const data = await getJson(`/api/sessions${query}`);
  renderSessions(data);

  const params = new URLSearchParams(window.location.search);
  const requested = params.get("session");
  const firstSession = requested || data.sessions?.[0]?.key;
  if (firstSession) {
    const node = [...$("sessions").querySelectorAll("li")].find(
      (li) => li.dataset.session === String(firstSession),
    );
    if (node) node.classList.add("active");
    await loadSession(firstSession);
  }
}

async function loadSession(sessionKey) {
  currentSession = sessionKey;
  const params = new URLSearchParams({ session: sessionKey });
  const target = getActiveTarget();
  if (target) params.set("target", target);
  currentData = await getJson(`/api/session?${params.toString()}`);
  resetSessionState(currentData);
  renderEvents();
  await ensureModeInitialized();
}

function applyUrlState() {
  const params = new URLSearchParams(window.location.search);
  const target = params.get("target");
  if (target) $("targetPath").value = target;
}

function attachHandlers() {
  $("loadBtn").addEventListener("click", () => {
    loadSessions().catch((err) => alert(err.message));
  });
  $("modeSelect").addEventListener("change", () => {
    ensureModeInitialized().catch((err) => alert(err.message));
  });
  $("filterInput").addEventListener("input", renderEvents);
  $("loadMoreBtn").addEventListener("click", () => {
    loadMoreEvents().catch((err) => alert(err.message));
  });
}

async function main() {
  applyUrlState();
  attachHandlers();
  await loadSessions();
}

main().catch((error) => {
  alert(error.message || String(error));
});
