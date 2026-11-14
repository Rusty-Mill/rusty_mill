// No logic here: render what the backend says, send the user's click back.
// Text is always set with textContent, never as HTML.
const { invoke } = window.__TAURI__.core;

// Theme: follow the system until the user picks one; the pick is remembered.
const media = window.matchMedia("(prefers-color-scheme: light)");
// Storage can be blocked; fall back to the system setting rather than failing to load.
function saved() {
  try { return localStorage.getItem("theme"); } catch { return null; }
}
const theme = () => saved() ?? (media.matches ? "light" : "dark");
function applyTheme() {
  const t = theme();
  document.documentElement.dataset.theme = t;
  $("theme").textContent = t === "light" ? "Dark theme" : "Light theme";
}
const $ = (id) => document.getElementById(id);
const LABEL = { absent: "not enabled", enabled: "enabled", stale: "stale", edited: "edited", foreign: "foreign" };
let lastView = { rows: [], tools: [], skipped: [] };
let pending = null; // { entry, enable } shown in the dialog

function el(tag, cls, text) {
  const n = document.createElement(tag);
  if (cls) n.className = cls;
  if (text !== undefined) n.textContent = text;
  return n;
}

function group(parent, title, items, refused) {
  if (items.length === 0) return;
  const g = el("div", refused ? "group refused" : "group");
  g.appendChild(el("div", "label", title));
  const rows = el("div", "rows");
  for (const { p, a } of items) {
    const r = el("div", "pr");
    r.appendChild(el("span", `k ${a.kind}`, a.kind));
    r.appendChild(el("span", "", p.tool));
    const detail = a.source ? `${a.path} → ${a.source} (${a.method})` : a.reason ? a.reason : a.path;
    r.appendChild(el("span", "d", detail));
    rows.appendChild(r);
  }
  g.appendChild(rows);
  parent.appendChild(g);
}

function showError(e) {
  $("error").textContent = String(e);
  $("error").hidden = false;
}

function stateCell(state) {
  const s = el("span", `state ${state}`);
  s.appendChild(el("span", "dot"));
  s.appendChild(document.createTextNode(LABEL[state] ?? state));
  return s;
}

// One button per row: Enable when nothing is enabled, else Disable.
const isOn = (row) => row.cells.some((c) => c.state === "enabled");
const needsLook = (row) => row.cells.some((c) => ["stale", "edited", "foreign"].includes(c.state));
const FILTERS = [
  ["all", "All", () => true],
  ["on", "Enabled", isOn],
  ["off", "Not enabled", (r) => !isOn(r)],
  ["look", "Needs a look", needsLook],
];
let filter = "all";

function actionFor(row) {
  const any = isOn(row);
  const btn = el("button", any ? "" : "primary", any ? "Disable…" : "Enable…");
  btn.addEventListener("click", () => preview(row.id, !any));
  return btn;
}

// Search only narrows what is shown; the backend owns every other decision.
function render() {
  const view = lastView;
  const q = $("q").value.trim().toLowerCase();
  const pick = FILTERS.find((f) => f[0] === filter)[2];
  const rows = view.rows.filter((r) => pick(r) && `${r.id} ${r.source ?? ""}`.toLowerCase().includes(q));
  const chips = $("chips");
  chips.replaceChildren();
  for (const [key, label, test] of FILTERS) {
    const b = el("button", "chip");
    b.setAttribute("aria-pressed", String(key === filter));
    b.appendChild(document.createTextNode(label));
    b.appendChild(el("b", "", String(view.rows.filter(test).length)));
    b.addEventListener("click", () => { filter = key; render(); });
    chips.appendChild(b);
  }
  const head = $("grid").tHead;
  head.replaceChildren();
  const hr = head.insertRow();
  ["Skill", ...view.tools, ""].forEach((t, i, all) => {
    const th = el("th", i === all.length - 1 ? "act" : "", t);
    hr.appendChild(th);
  });
  const body = $("grid").tBodies[0];
  body.replaceChildren();
  for (const row of rows) {
    const tr = body.insertRow();
    const name = tr.insertCell();
    name.className = "id";
    name.appendChild(document.createTextNode(row.id));
    name.appendChild(el("small", "", row.source));
    for (const c of row.cells) tr.insertCell().appendChild(stateCell(c.state));
    const act = tr.insertCell();
    act.className = "act";
    act.appendChild(actionFor(row));
  }
  $("grid").hidden = rows.length === 0;
  $("empty").hidden = rows.length !== 0;
  $("sub").textContent = `${view.rows.length} skills across ${view.tools.join(", ")}`;
  const skipped = view.skipped.map((s) => `${s.path}: ${s.reason}`);
  $("skipped").textContent = skipped.length ? `Skipped: ${skipped.join("; ")}` : "";
  $("skipped").hidden = skipped.length === 0;
}

async function refresh() {
  try {
    $("error").hidden = true;
    lastView = await invoke("list");
    render();
  } catch (e) {
    showError(e);
  }
}

async function preview(entry, enable) {
  try {
    const plans = await invoke("plan_change", { entry, verb: enable ? "enable" : "disable" });
    pending = { entry, enable };
    const body = $("plan-body");
    body.replaceChildren();
    const apply = [];
    const refused = [];
    for (const p of plans) {
      for (const a of p.actions) (a.kind === "refuse" || p.refused ? refused : apply).push({ p, a });
    }
    $("plan-title").textContent = `${enable ? "Enable" : "Disable"} ${entry}`;
    group(body, "Will apply", apply, false);
    group(body, "Refused, left untouched", refused, true);
    if (plans.every((p) => p.noop)) body.appendChild(el("p", "dim", "Nothing to do."));
    const n = (k, w) => `${k} ${w}${k === 1 ? "" : "s"}`;
    const nRefused = refused.filter((x) => x.a.kind === "refuse").length;
    $("plan-summary").textContent = `${n(apply.length, "change")} will apply and ${nRefused} refused. Nothing has touched disk yet.`;
    $("plan-apply").disabled = apply.length === 0;
    $("plan-apply").textContent = `Apply ${n(apply.length, "change")}`;
    $("plan").showModal();
  } catch (e) {
    showError(e);
  }
}

$("q").addEventListener("input", render);
$("clear").addEventListener("click", () => { $("q").value = ""; filter = "all"; render(); });
$("theme").addEventListener("click", () => { const next = theme() === "light" ? "dark" : "light";
  try { localStorage.setItem("theme", next); } catch { /* not remembered, still applied below */ }
  document.documentElement.dataset.theme = next;
  $("theme").textContent = next === "light" ? "Dark theme" : "Light theme"; });
media.addEventListener("change", applyTheme);
$("plan-cancel").addEventListener("click", () => $("plan").close());
$("plan-apply").addEventListener("click", async () => {
  const { entry, enable } = pending;
  $("plan").close();
  try {
    await invoke("apply_change", { entry, verb: enable ? "enable" : "disable" });
  } catch (e) {
    showError(e);
  }
  refresh();
});

applyTheme();
refresh();
