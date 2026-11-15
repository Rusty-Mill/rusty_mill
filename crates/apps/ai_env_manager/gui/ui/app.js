// No logic here: render what the backend says, send the user's click back.
// Text is always set with textContent, never as HTML.
const { invoke } = window.__TAURI__.core;

// Theme: follow the system until the user picks one. The pick is kept in memory, and
// persisted when storage allows it (it can be blocked, which must not break the toggle).
const media = window.matchMedia("(prefers-color-scheme: light)");
function stored() {
  try { return localStorage.getItem("theme"); } catch { return null; }
}
let chosen = stored();
const theme = () => chosen ?? (media.matches ? "light" : "dark");
function applyTheme() {
  const t = theme();
  document.documentElement.dataset.theme = t;
  $("theme").textContent = t === "light" ? "Dark theme" : "Light theme";
}
function toggleTheme() {
  chosen = theme() === "light" ? "dark" : "light";
  try { localStorage.setItem("theme", chosen); } catch { /* kept in memory only */ }
  applyTheme();
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
  const box = el("span", "btns");
  const files = el("button", "", "Files…");
  files.addEventListener("click", () => openEditor(row));
  box.appendChild(files);
  box.appendChild(toggleFor(row));
  return box;
}

function toggleFor(row) {
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
$("theme").addEventListener("click", toggleTheme);
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

// ---- Editor: the backend decides what is editable and what a save does. ----
// ed = { entry, tools, tool, files, path, hash, text, ownership, editable, mode }; mode is view | edit | review.
let ed = null;
const edTool = () => ($("ed-place").value === "" ? null : $("ed-place").value);
const EDIT_NOTE = {
  source: "This is the source folder. A save changes the original.",
  copy: "This is a copy we made. A save marks it edited, and it is never replaced afterwards.",
  foreign: "Not ours: read-only.",
};

// The list does not say how a tool was installed: linked ones change with a source save, copied
// ones go stale until the next apply. The wording is true either way.
const sharedNote = () => (ed.ownership === "source" && ed.users.length > 1 ? ` Enabled for ${ed.users.join(", ")}: linked ones change with this save, copied ones go stale until you apply.` : "");
const FOOT = "Nothing is written until you confirm the diff.";
function edFoot(text = FOOT) {
  $("ed-foot").textContent = text;
}

function edError(e) {
  $("ed-error").textContent = e ? String(e) : "";
  $("ed-error").hidden = !e;
}

function openEditor(row) {
  ed = { entry: row.id, files: [], path: null, mode: "view", users: row.cells.filter((c) => c.state === "enabled").map((c) => c.tool) };
  closeWarn();
  edFoot();
  $("ed-title").textContent = row.id;
  const sel = $("ed-place");
  sel.replaceChildren();
  const src = el("option", "", "source folder");
  src.value = "";
  sel.appendChild(src);
  for (const t of lastView.tools) {
    const o = el("option", "", t);
    o.value = t;
    sel.appendChild(o);
  }
  edShow();
  $("editor").showModal();
  loadFiles();
}

async function loadFiles() {
  edError(null);
  ed.path = null;
  try {
    const v = await invoke("files", { entry: ed.entry, tool: edTool() });
    ed.files = v.files;
    ed.ownership = v.ownership;
  } catch (e) {
    ed.files = [];
    edError(e);
  }
  edShow();
}

async function openFile(path) {
  closeWarn();
  edError(null);
  try {
    const v = await invoke("file", { entry: ed.entry, tool: edTool(), path });
    Object.assign(ed, { path, text: v.text, hash: v.hash, ownership: v.ownership, mode: "view" });
    ed.editable = (ed.files.find((f) => f.path === path) ?? {}).editable === true;
  } catch (e) {
    edError(e);
  }
  edShow();
}

// Unsaved text is never dropped silently: anything that would lose it asks first.
const dirty = () => ed.mode !== "view" && !ed.pushing && $("ed-text").value !== ed.text;
let afterDrop = null;
function guard(action, what) {
  if (!dirty()) return action();
  afterDrop = action;
  $("ed-warn-text").textContent = `You have unsaved changes. ${what} would discard them.`;
  $("ed-warn").hidden = false;
  $("ed-drop").focus();
}
function closeWarn() {
  afterDrop = null;
  $("ed-warn").hidden = true;
}
$("ed-keep").addEventListener("click", closeWarn);
$("ed-drop").addEventListener("click", () => {
  const go = afterDrop;
  closeWarn();
  if (go) go();
});

// Push is offered for a tool's copy the list reports as edited; the backend re-checks all of it.
const canPush = () => {
  const tool = edTool();
  const row = lastView.rows.find((r) => r.id === ed.entry);
  return tool !== null && ed.ownership === "copy" && (row?.cells.find((c) => c.tool === tool)?.state ?? "") === "edited";
};

function edShow() {
  const list = $("ed-files");
  list.replaceChildren();
  for (const f of ed.files) {
    const b = el("button", f.editable ? "" : "ro", f.path);
    if (f.path === ed.path) b.setAttribute("aria-current", "true");
    b.addEventListener("click", () => guard(() => openFile(f.path), "Opening another file"));
    list.appendChild(el("li")).appendChild(b);
  }
  const open = ed.path !== null;
  const m = ed.mode;
  $("ed-code").hidden = !(open && (m === "view" || m === "edit"));
  $("ed-view").hidden = !(open && m === "view");
  $("ed-text").hidden = m !== "edit";
  $("ed-hl").hidden = m !== "edit";
  $("ed-diff").hidden = m !== "review";
  if (open && m === "view") {
    Code.paint($("ed-view"), ed.text, ed.path);
    Code.number($("ed-gutter"), ed.text);
  }
  $("ed-note").textContent = !open
    ? ed.files.length ? "Pick a file." : ""
    : `${ed.path}. ${EDIT_NOTE[ed.ownership] ?? ""}${ed.editable ? sharedNote() : " Read-only."}`;
  $("ed-edit").hidden = !(open && m === "view" && ed.editable);
  $("ed-push").hidden = !(m === "view" && canPush());
  $("ed-reload").hidden = m === "view";
  $("ed-back").hidden = m !== "review";
  $("ed-review").hidden = m !== "edit";
  $("ed-save").hidden = m !== "review";
  $("ed-place").disabled = m !== "view";
}

// Code view: the textarea (or the read-only view) is the scroller; the colour layer and the gutter follow it.
function codeSync() {
  const t = $("ed-text").value;
  Code.paint($("ed-hl"), `${t}\n`, ed.path);
  Code.number($("ed-gutter"), t);
  follow($("ed-text"));
}
function follow(from) {
  $("ed-hl").scrollTop = from.scrollTop;
  $("ed-hl").scrollLeft = from.scrollLeft;
  $("ed-gutter").scrollTop = from.scrollTop;
}
let queued = false;
$("ed-text").addEventListener("input", () => {
  if (queued) return;
  queued = true;
  requestAnimationFrame(() => { queued = false; codeSync(); });
});
$("ed-text").addEventListener("scroll", (e) => follow(e.target));
$("ed-view").addEventListener("scroll", (e) => follow(e.target));

$("ed-place").addEventListener("change", loadFiles); // the select is disabled while editing
$("ed-close").addEventListener("click", () => guard(() => $("editor").close(), "Closing"));
$("editor").addEventListener("cancel", (e) => {
  if (!dirty()) return;
  e.preventDefault();
  guard(() => $("editor").close(), "Closing");
});
$("ed-reload").addEventListener("click", () => openFile(ed.path)); // also the way out of "changed on disk"
$("ed-edit").addEventListener("click", () => {
  ed.mode = "edit";
  edFoot();
  $("ed-text").value = ed.text;
  edShow();
  codeSync();
  $("ed-text").focus();
});
$("ed-back").addEventListener("click", () => {
  ed.mode = ed.pushing ? "view" : "edit";
  ed.reviewed = null;
  ed.pushing = null;
  edFoot();
  edShow();
});
// What the diff showed is exactly what Confirm saves: the review takes a snapshot, and the
// snapshot is what apply_edit gets. The box is frozen while a request is out.
$("ed-review").addEventListener("click", async () => {
  edError(null);
  const snap = { entry: ed.entry, tool: edTool(), path: ed.path, text: $("ed-text").value, hash: ed.hash };
  $("ed-text").readOnly = true;
  try {
    const p = await invoke("preview_edit", snap);
    if (ed.mode !== "edit" || ed.entry !== snap.entry || edTool() !== snap.tool || ed.path !== snap.path) return; // the user moved on meanwhile
    const box = $("ed-diff");
    box.replaceChildren();
    for (const d of p.diff) box.appendChild(el("div", d.kind, `${{ added: "+ ", removed: "- ", same: "  " }[d.kind]}${d.text}`));
    const changed = p.diff.filter((d) => d.kind !== "same").length;
    $("ed-foot").textContent = changed === 0
      ? "No changes."
      : `${p.marks_edited ? "Saving marks this copy edited. " : ""}Nothing is written until you confirm.${sharedNote()}`;
    $("ed-save").disabled = changed === 0;
    ed.reviewed = snap;
    ed.mode = "review";
    edShow();
  } catch (e) {
    edError(e);
  } finally {
    $("ed-text").readOnly = false;
  }
});
// Push: copy a tool's edits back to the source. Like a save, the diff shown is what Confirm sends.
$("ed-push").addEventListener("click", async () => {
  edError(null);
  const tool = edTool();
  try {
    const p = await invoke("preview_push", { entry: ed.entry, tool, path: null });
    const box = $("ed-diff");
    box.replaceChildren();
    for (const f of p.files) {
      box.appendChild(el("div", "same", `── ${f.path}`));
      for (const d of f.diff) box.appendChild(el("div", d.kind, `${{ added: "+ ", removed: "- ", same: "  " }[d.kind]}${d.text}`));
    }
    for (const l of p.left) box.appendChild(el("div", "same", `left alone: ${l.path} (${l.why})`));
    $("ed-foot").textContent = "Pushing overwrites the source, including changes made there since this copy was taken. Nothing is written until you confirm.";
    $("ed-save").disabled = p.files.length === 0;
    ed.pushing = { entry: ed.entry, tool, bases: p.files.map(({ path, source_hash, copy_hash }) => ({ path, source_hash, copy_hash })) };
    ed.mode = "review";
    edShow();
  } catch (e) {
    edError(e);
  }
});
async function applyPush() {
  const snap = ed.pushing;
  $("ed-save").disabled = true;
  let done;
  try {
    done = await invoke("apply_push", snap);
  } catch (e) {
    edError(e);
    $("ed-save").disabled = false;
    return;
  }
  ed.pushing = null;
  ed.mode = "view";
  $("ed-foot").textContent = done.remarked
    ? "Pushed. The copy matches the source again."
    : `Pushed. The copy is still edited: ${done.left.map((l) => l.path).join(", ")} differ.`;
  await refresh();
  await loadFiles();
}
$("ed-save").addEventListener("click", async () => {
  edError(null);
  if (ed.pushing) return applyPush();
  const snap = ed.reviewed;
  if (!snap) return;
  $("ed-save").disabled = true;
  try {
    // The text stays in the box until the backend says it was saved.
    await invoke("apply_edit", snap);
  } catch (e) {
    edError(e);
    $("ed-save").disabled = false;
    return;
  }
  ed.reviewed = null;
  ed.mode = "view";
  $("ed-foot").textContent = "Saved.";
  await openFile(snap.path);
  refresh();
});

applyTheme();
refresh();
