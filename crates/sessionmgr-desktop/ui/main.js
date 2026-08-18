import { Grid, MIN_WEIGHT } from "./grid.js";

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const ATTACHABLE = new Set(["created", "running", "needs-input"]);
const STATUS_CLASS = new Set(["running", "needs-input", "errored", "crashed"]);
const POLL_INTERVAL_MS = 2000;

/** @type {Map<string, object>} id -> SessionSummary */
let sessions = new Map();
/** @type {Map<string, object>} id -> pane record */
const panes = new Map();
/** Row-major pane order, independent of `panes`' iteration order, so a
 *  grid rebuild is stable even if a re-poll's JSON key order isn't. */
let paneOrder = [];
let focusedId = null;
let grid = new Grid(0);
let everLaidOut = false;
/** Filled in by `layout()`; `applyWeights()` only ever touches these. */
let rowEls = [];
let cellEls = []; // cellEls[r][c] -> element (pane.el or a placeholder)

const gridEl = document.getElementById("grid");
const appEl = document.getElementById("app");
const sidebarListEl = document.getElementById("session-list");
const statusTextEl = document.getElementById("status-text");

function setStatus(text) {
  statusTextEl.textContent = text;
}

function b64ToBytes(b64) {
  const bin = atob(b64);
  const bytes = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
  return bytes;
}

// -- fuzzy match, ported from sessionmgr-tui::app::fuzzy_match ------------

function fuzzyMatch(query, label) {
  const l = label.toLowerCase();
  const q = query.toLowerCase();
  let li = 0;
  for (const qc of q) {
    let found = false;
    while (li < l.length) {
      const lc = l[li++];
      if (lc === qc) {
        found = true;
        break;
      }
    }
    if (!found) return false;
  }
  return true;
}

// -- session list polling / sidebar ---------------------------------------

async function refreshSessions() {
  let list;
  try {
    list = await invoke("session_list");
  } catch (e) {
    setStatus(`could not reach the daemon: ${e}`);
    return;
  }
  sessions = new Map(list.map((s) => [s.id, s]));
  renderSidebar();

  const attachableIds = new Set(list.filter((s) => ATTACHABLE.has(s.status)).map((s) => s.id));

  let changed = false;
  for (const id of [...paneOrder]) {
    if (!attachableIds.has(id)) {
      await removePane(id);
      changed = true;
    }
  }
  for (const s of list) {
    if (!attachableIds.has(s.id)) continue;
    if (!panes.has(s.id)) {
      await addPane(s);
      changed = true;
    } else {
      // Refreshes the pane's own name/status labels every poll, not
      // just on creation -- the sidebar already does this via
      // `renderSidebar()` reading straight from `sessions`, but a
      // pane's titlebar is only ever set once, at `addPane()`, unless
      // told again here.
      updatePaneHeader(s.id);
    }
  }
  // `changed` alone misses the "still zero sessions" case on first boot
  // -- nothing was ever added or removed, so `layout()` (the only thing
  // that toggles `#app.empty`) would otherwise never run at all and the
  // empty-state message would never appear.
  if (changed || !everLaidOut) {
    everLaidOut = true;
    layout();
  }
  applyWeights();
}

function renderSidebar() {
  sidebarListEl.innerHTML = "";
  for (const s of sessions.values()) {
    const li = document.createElement("li");
    if (s.id === focusedId) li.classList.add("focused");
    const label = document.createElement("div");
    label.className = "sid";
    label.textContent = s.name ? `${s.name}` : s.id;
    const status = document.createElement("div");
    status.className = "status" + (STATUS_CLASS.has(s.status) ? ` ${s.status}` : "");
    status.textContent = `${s.status} · ${s.kind}`;
    li.appendChild(label);
    li.appendChild(status);
    li.addEventListener("click", () => {
      if (panes.has(s.id)) focus(s.id);
      else setStatus(`${s.id} is ${s.status}; not live`);
    });
    sidebarListEl.appendChild(li);
  }
}

// -- panes ------------------------------------------------------------------

async function addPane(summary) {
  const el = document.createElement("div");
  el.className = "pane";

  const titlebar = document.createElement("div");
  titlebar.className = "pane-titlebar";
  const nameEl = document.createElement("span");
  nameEl.className = "pane-name";
  const statusEl = document.createElement("span");
  statusEl.className = "pane-status";
  titlebar.appendChild(nameEl);
  titlebar.appendChild(statusEl);

  const toolbar = document.createElement("span");
  toolbar.style.marginLeft = "auto";
  toolbar.style.display = "flex";
  toolbar.style.gap = "4px";
  const btn = (label, title, onClick) => {
    const b = document.createElement("button");
    b.textContent = label;
    b.title = title;
    b.style.cssText =
      "background:none;border:1px solid var(--border);color:var(--text-dim);border-radius:3px;cursor:pointer;font-size:10px;padding:1px 5px;";
    b.addEventListener("click", (ev) => {
      ev.stopPropagation();
      onClick();
    });
    toolbar.appendChild(b);
  };
  btn("diff", "Toggle git diff", () => actionToggleDiff(summary.id));
  btn("rename", "Rename session", () => actionRename(summary.id));
  btn("fork", "Fork session", () => actionFork(summary.id));
  btn("switch", "Switch agent", () => actionSwitchAgent(summary.id));
  btn("×", "Close session", () => actionClose(summary.id));
  titlebar.appendChild(toolbar);
  titlebar.addEventListener("click", () => focus(summary.id));

  const body = document.createElement("div");
  body.className = "pane-body";

  const termEl = document.createElement("div");
  termEl.className = "pane-terminal";
  body.appendChild(termEl);

  const diffEl = document.createElement("div");
  diffEl.className = "pane-diff";
  const diffFilesEl = document.createElement("div");
  diffFilesEl.className = "diff-files";
  const diffTextEl = document.createElement("pre");
  diffTextEl.className = "diff-text";
  diffEl.appendChild(diffFilesEl);
  diffEl.appendChild(diffTextEl);
  body.appendChild(diffEl);

  el.appendChild(titlebar);
  el.appendChild(body);
  el.addEventListener("mousedown", () => focus(summary.id));

  const term = new window.Terminal({
    scrollback: 5000,
    fontSize: 13,
    fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace",
    theme: {
      background: "#0d0f12",
      foreground: "#d8dee4",
      cursor: "#4fa3d1",
      selectionBackground: "#33404d",
    },
  });
  const fitAddon = new window.FitAddon.FitAddon();
  term.loadAddon(fitAddon);
  term.open(termEl);

  const pane = {
    id: summary.id,
    el,
    nameEl,
    statusEl,
    term,
    fitAddon,
    diffMode: false,
    diffFiles: [],
    diffFilesEl,
    diffTextEl,
    diffSelectedPath: null,
    lastRows: 0,
    lastCols: 0,
  };
  panes.set(summary.id, pane);
  paneOrder.push(summary.id);
  updatePaneHeader(summary.id);

  term.onData((data) => {
    const bytes = Array.from(new TextEncoder().encode(data));
    invoke("send_input", { id: summary.id, data: bytes }).catch(() => {});
  });

  try {
    await invoke("attach_session", { id: summary.id });
  } catch (e) {
    setStatus(`could not attach to ${summary.id}: ${e}`);
  }

  if (!focusedId) focus(summary.id);
}

async function removePane(id) {
  const p = panes.get(id);
  if (!p) return;
  invoke("detach_session", { id }).catch(() => {});
  p.term.dispose();
  p.el.remove();
  panes.delete(id);
  paneOrder = paneOrder.filter((x) => x !== id);
  if (focusedId === id) {
    focusedId = null;
    if (paneOrder.length) focus(paneOrder[0]);
  }
}

function updatePaneHeader(id) {
  const p = panes.get(id);
  const s = sessions.get(id);
  if (!p) return;
  p.nameEl.textContent = s?.name || id;
  p.statusEl.textContent = s?.status || "";
}

function focus(id) {
  if (focusedId && panes.has(focusedId)) {
    panes.get(focusedId).el.classList.remove("focused");
  }
  focusedId = id;
  const p = panes.get(id);
  if (p) {
    p.el.classList.add("focused");
    p.term.focus();
  }
  renderSidebar();
}

// -- layout: row-major grid, shared row/col weights, drag-resizable ------

function layout() {
  const { rows, cols } = grid;
  if (paneOrder.length !== grid.rows * grid.cols || grid.rows === 0) {
    grid = new Grid(paneOrder.length);
  }
  gridEl.innerHTML = "";
  rowEls = [];
  cellEls = [];
  appEl.classList.toggle("empty", paneOrder.length === 0);

  gridEl.style.display = "flex";
  gridEl.style.flexDirection = "column";

  const { rows: r2, cols: c2 } = grid;
  for (let r = 0; r < r2; r++) {
    const rowEl = document.createElement("div");
    rowEl.style.display = "flex";
    rowEl.style.gap = "4px";
    rowEl.style.minHeight = "0";
    rowEls.push(rowEl);
    const rowCells = [];
    for (let c = 0; c < c2; c++) {
      if (c > 0) {
        rowEl.appendChild(colResizer(c - 1));
      }
      const idx = r * c2 + c;
      let cell;
      if (idx < paneOrder.length) {
        cell = panes.get(paneOrder[idx]).el;
      } else {
        cell = document.createElement("div");
      }
      cell.style.minWidth = "0";
      cell.style.minHeight = "0";
      rowEl.appendChild(cell);
      rowCells.push(cell);
    }
    cellEls.push(rowCells);
    gridEl.appendChild(rowEl);
    if (r < r2 - 1) {
      gridEl.appendChild(rowResizer(r));
    }
  }
  applyWeights();
  fitAllPanes();
}

function applyWeights() {
  for (let r = 0; r < rowEls.length; r++) {
    rowEls[r].style.flexGrow = String(grid.rowWeights[r] ?? 0);
    rowEls[r].style.flexBasis = "0";
  }
  for (let r = 0; r < cellEls.length; r++) {
    for (let c = 0; c < cellEls[r].length; c++) {
      cellEls[r][c].style.flexGrow = String(grid.colWeights[c] ?? 0);
      cellEls[r][c].style.flexBasis = "0";
    }
  }
}

function colResizer(boundary) {
  const el = document.createElement("div");
  el.className = "col-resizer";
  wireDrag(el, (deltaPx, containerPx) => {
    const [i, j] = [boundary, boundary + 1];
    const sumWeight = grid.colWeights[i] + grid.colWeights[j];
    const deltaWeight = (deltaPx / containerPx) * sumWeight;
    grid.dragColBoundary(i, grid.colWeights[i] + deltaWeight);
    applyWeights();
  });
  return el;
}

function rowResizer(boundary) {
  const el = document.createElement("div");
  el.className = "row-resizer";
  wireDrag(
    el,
    (deltaPx, containerPx) => {
      const [i, j] = [boundary, boundary + 1];
      const sumWeight = grid.rowWeights[i] + grid.rowWeights[j];
      const deltaWeight = (deltaPx / containerPx) * sumWeight;
      grid.dragRowBoundary(i, grid.rowWeights[i] + deltaWeight);
      applyWeights();
    },
    true,
  );
  return el;
}

function wireDrag(handle, onMove, vertical = false) {
  handle.addEventListener("pointerdown", (e) => {
    e.preventDefault();
    const containerPx = vertical ? gridEl.clientHeight : gridEl.clientWidth;
    const start = vertical ? e.clientY : e.clientX;
    handle.setPointerCapture(e.pointerId);
    const onPointerMove = (ev) => {
      const now = vertical ? ev.clientY : ev.clientX;
      onMove(now - start, containerPx || 1);
    };
    const onPointerUp = () => {
      handle.removeEventListener("pointermove", onPointerMove);
      handle.removeEventListener("pointerup", onPointerUp);
      fitAllPanes();
    };
    handle.addEventListener("pointermove", onPointerMove);
    handle.addEventListener("pointerup", onPointerUp);
  });
}

function fitAllPanes() {
  for (const p of panes.values()) {
    try {
      p.fitAddon.fit();
    } catch {
      continue;
    }
    const rows = p.term.rows;
    const cols = p.term.cols;
    if (rows !== p.lastRows || cols !== p.lastCols) {
      p.lastRows = rows;
      p.lastCols = cols;
      invoke("send_resize", { id: p.id, rows, cols }).catch(() => {});
    }
  }
}

let resizeTimer = null;
window.addEventListener("resize", () => {
  clearTimeout(resizeTimer);
  resizeTimer = setTimeout(fitAllPanes, 120);
});

// -- git diff pane ------------------------------------------------------

async function actionToggleDiff(id) {
  const p = panes.get(id);
  if (!p) return;
  if (p.diffMode) {
    p.el.classList.remove("diff-mode");
    p.diffMode = false;
    setTimeout(fitAllPanes, 0);
    return;
  }
  const s = sessions.get(id);
  if (s?.kind === "plain-terminal") {
    setStatus("a plain terminal session has no git workspace");
    return;
  }
  try {
    const files = await invoke("git_status", { id });
    p.diffFiles = files;
    renderDiffFiles(p);
    if (files.length) await selectDiffFile(p, files[0].path);
    p.el.classList.add("diff-mode");
    p.diffMode = true;
  } catch (e) {
    setStatus(`git status failed: ${e}`);
  }
}

function renderDiffFiles(p) {
  p.diffFilesEl.innerHTML = "";
  for (const f of p.diffFiles) {
    const row = document.createElement("div");
    row.textContent = `${f.status} ${f.path}`;
    if (f.path === p.diffSelectedPath) row.classList.add("selected");
    row.addEventListener("click", () => selectDiffFile(p, f.path));
    p.diffFilesEl.appendChild(row);
  }
}

async function selectDiffFile(p, path) {
  try {
    const diff = await invoke("git_diff", { id: p.id, path });
    p.diffTextEl.textContent = diff || "(no changes)";
    p.diffSelectedPath = path;
    renderDiffFiles(p);
  } catch (e) {
    setStatus(`git diff failed: ${e}`);
  }
}

// -- session actions ------------------------------------------------------

async function actionNewSession() {
  openPrompt("New session -- repository path", "", async (repo) => {
    if (!repo.trim()) {
      setStatus("new session: repo path cannot be empty");
      return;
    }
    try {
      const id = await invoke("session_new", { repo: repo.trim(), agent: null });
      setStatus(`created ${id}`);
      await refreshSessions();
    } catch (e) {
      setStatus(`new session failed: ${e}`);
    }
  });
}

async function actionClose(id) {
  try {
    await invoke("session_close", { id });
    setStatus(`closed ${id}`);
    await refreshSessions();
  } catch (e) {
    setStatus(`close failed: ${e}`);
  }
}

async function actionRename(id) {
  const current = sessions.get(id)?.name || "";
  openPrompt("Rename session", current, async (name) => {
    try {
      await invoke("session_rename", { id, name: name.trim() ? name.trim() : null });
      setStatus(`renamed ${id}`);
      await refreshSessions();
    } catch (e) {
      setStatus(`rename failed: ${e}`);
    }
  });
}

async function actionFork(id) {
  try {
    const forked = await invoke("session_fork", { id });
    setStatus(`forked ${id} -> ${forked}`);
    await refreshSessions();
  } catch (e) {
    setStatus(`fork failed: ${e}`);
  }
}

async function actionSwitchAgent(id) {
  openPrompt("Switch agent -- claude/codex/gemini", "", async (agent) => {
    try {
      const switched = await invoke("session_switch_agent", { id, agent: agent.trim() });
      setStatus(`${id} switched agent -> ${switched}`);
      await refreshSessions();
    } catch (e) {
      setStatus(`switch-agent failed: ${e}`);
    }
  });
}

// -- command palette --------------------------------------------------------

const paletteOverlay = document.getElementById("palette-overlay");
const paletteInput = document.getElementById("palette-input");
const paletteListEl = document.getElementById("palette-list");
let paletteItems = [];
let paletteSelected = 0;

function buildPaletteItems() {
  const items = [{ label: "New session...", run: actionNewSession }];
  if (focusedId && panes.has(focusedId)) {
    const id = focusedId;
    items.push({ label: "Close focused session", run: () => actionClose(id) });
    items.push({ label: "Rename focused session...", run: () => actionRename(id) });
    items.push({ label: "Fork focused session", run: () => actionFork(id) });
    items.push({ label: "Switch focused session's agent...", run: () => actionSwitchAgent(id) });
    items.push({ label: "Toggle git diff for focused session", run: () => actionToggleDiff(id) });
  }
  for (const id of paneOrder) {
    if (id === focusedId) continue;
    const s = sessions.get(id);
    const label = s?.name || id;
    items.push({ label: `Focus: ${label}`, run: () => focus(id) });
  }
  return items;
}

function openPalette() {
  paletteItems = buildPaletteItems();
  paletteSelected = 0;
  paletteInput.value = "";
  renderPalette();
  paletteOverlay.classList.remove("hidden");
  paletteInput.focus();
}

function closePalette() {
  paletteOverlay.classList.add("hidden");
}

function renderPalette() {
  const query = paletteInput.value;
  const filtered = paletteItems.filter((i) => fuzzyMatch(query, i.label));
  paletteListEl.innerHTML = "";
  filtered.forEach((item, i) => {
    const li = document.createElement("li");
    li.textContent = item.label;
    if (i === paletteSelected) li.classList.add("selected");
    li.addEventListener("mousedown", (e) => {
      e.preventDefault();
      closePalette();
      item.run();
    });
    paletteListEl.appendChild(li);
  });
  return filtered;
}

paletteInput.addEventListener("input", () => {
  paletteSelected = 0;
  renderPalette();
});

paletteInput.addEventListener("keydown", (e) => {
  const filtered = paletteItems.filter((i) => fuzzyMatch(paletteInput.value, i.label));
  if (e.key === "Escape") {
    e.preventDefault();
    closePalette();
  } else if (e.key === "ArrowDown") {
    e.preventDefault();
    paletteSelected = Math.min(paletteSelected + 1, filtered.length - 1);
    renderPalette();
  } else if (e.key === "ArrowUp") {
    e.preventDefault();
    paletteSelected = Math.max(paletteSelected - 1, 0);
    renderPalette();
  } else if (e.key === "Enter") {
    e.preventDefault();
    const item = filtered[paletteSelected];
    closePalette();
    if (item) item.run();
  }
});

paletteOverlay.addEventListener("mousedown", (e) => {
  if (e.target === paletteOverlay) closePalette();
});

// -- prompt overlay -----------------------------------------------------

const promptOverlay = document.getElementById("prompt-overlay");
const promptTitleEl = document.getElementById("prompt-title");
const promptInput = document.getElementById("prompt-input");
let promptOnSubmit = null;

function openPrompt(title, initial, onSubmit) {
  promptTitleEl.textContent = title;
  promptInput.value = initial;
  promptOnSubmit = onSubmit;
  promptOverlay.classList.remove("hidden");
  promptInput.focus();
  promptInput.select();
}

function closePrompt() {
  promptOverlay.classList.add("hidden");
  promptOnSubmit = null;
}

promptInput.addEventListener("keydown", (e) => {
  if (e.key === "Escape") {
    e.preventDefault();
    closePrompt();
  } else if (e.key === "Enter") {
    e.preventDefault();
    const value = promptInput.value;
    const onSubmit = promptOnSubmit;
    closePrompt();
    if (onSubmit) onSubmit(value);
  }
});

promptOverlay.addEventListener("mousedown", (e) => {
  if (e.target === promptOverlay) closePrompt();
});

// -- global keybindings ---------------------------------------------------

// Capture phase, deliberately: xterm.js's own keydown handler calls
// `stopPropagation()` on most combinations (so a terminal's own Ctrl
// bindings don't leak into page shortcuts), which would otherwise
// swallow Ctrl/Cmd+K and Ctrl/Cmd+N before a bubble-phase listener on
// `document` ever saw them. A capture-phase listener on `document` runs
// before the event reaches the focused pane's textarea at all.
document.addEventListener(
  "keydown",
  (e) => {
    const mod = e.metaKey || e.ctrlKey;
    if (!mod) return;
    if (e.key.toLowerCase() === "k") {
      e.preventDefault();
      e.stopPropagation();
      if (paletteOverlay.classList.contains("hidden")) openPalette();
      else closePalette();
    } else if (e.key.toLowerCase() === "n") {
      e.preventDefault();
      e.stopPropagation();
      actionNewSession();
    }
  },
  true,
);

document.getElementById("new-session-btn").addEventListener("click", actionNewSession);

// -- session events ----------------------------------------------------------

listen("session-event", (evt) => {
  const { id, event } = evt.payload;
  const p = panes.get(id);
  switch (event.type) {
    case "output": {
      if (p) p.term.write(b64ToBytes(event.data));
      break;
    }
    case "status": {
      const s = sessions.get(id);
      if (s) s.status = event.status;
      updatePaneHeader(id);
      renderSidebar();
      break;
    }
    case "exited": {
      setStatus(`session ${id} exited (${JSON.stringify(event.code)})`);
      break;
    }
    case "recovery-marker": {
      if (p) p.term.write("\r\n[reattached to a session that survived a manager restart]\r\n");
      break;
    }
  }
});

// -- boot -----------------------------------------------------------------

refreshSessions();
setInterval(refreshSessions, POLL_INTERVAL_MS);
