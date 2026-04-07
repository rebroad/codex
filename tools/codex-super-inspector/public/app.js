function $(id) {
  return document.getElementById(id);
}

let currentTarget = "";
let currentSession = "";
let currentData = null;

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

function isSupervisorInjected(event) {
  const meta = event?.parsed?._supervisor;
  return (
    event?.direction === "c2s" &&
    meta &&
    typeof meta === "object" &&
    meta.injectedBy === "supervisor"
  );
}

function renderEvent(event, idx) {
  const supervisorInjected = isSupervisorInjected(event);
  const chips = [
    `<span class="chip">#${idx + 1}</span>`,
    `<span class="chip">${escapeHtml(event.direction)}</span>`,
    supervisorInjected ? `<span class="chip chip-supervisor">supervisor</span>` : "",
    event.timestamp ? `<span class="chip">${escapeHtml(event.timestamp)}</span>` : "",
    `<span class="chip">line=${event.lineNo}</span>`,
    `<span class="chip">${escapeHtml(event.summary || "")}</span>`,
  ].join("");

  const classes = ["event-card", escapeHtml(event.direction)];
  if (supervisorInjected) classes.push("supervisor-injected");

  return (
    `<article class="${classes.join(" ")}">` +
    `<div class="event-head">${chips}</div>` +
    `<pre>${escapeHtml(formatBody(event))}</pre>` +
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

function renderEvents() {
  if (!currentData) {
    $("events").innerHTML = `<div class="empty">(select a session)</div>`;
    return;
  }
  const mode = selectedMode();
  const all = currentData.events[mode] || [];
  const events = filteredEvents(all);
  $("eventsMeta").textContent =
    `session=${currentData.session.key}\n` +
    `mode=${mode}\n` +
    `shown=${events.length} total=${all.length}\n` +
    `c2s=${currentData.totals.c2s} s2c=${currentData.totals.s2c} merged=${currentData.totals.merged}`;
  $("events").innerHTML = events.length
    ? events.map(renderEvent).join("")
    : `<div class="empty">(no events match current filter)</div>`;
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
  const target = $("targetPath").value.trim() || currentTarget;
  if (target) params.set("target", target);
  currentData = await getJson(`/api/session?${params.toString()}`);
  renderEvents();
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
  $("modeSelect").addEventListener("change", renderEvents);
  $("filterInput").addEventListener("input", renderEvents);
}

async function main() {
  applyUrlState();
  attachHandlers();
  await loadSessions();
}

main().catch((error) => {
  alert(error.message || String(error));
});
