//! The single-page web UI, served at `/`.
//!
//! Vanilla HTML/CSS/JS (no build step, no CDN): it loads a `.replay` — dropped,
//! picked, or one of the bundled samples — POSTs it to `/api/analyze`, and
//! renders the unified [`Analysis`](crate::pipeline::Analysis) across tabs:
//! an overview, the embedded 3D viewer, the scoring report, the skills table,
//! and the value-impact table. The two HTML views are self-contained documents
//! dropped into iframes, so their styles never collide with the shell.

/// The full index document.
pub const INDEX_HTML: &str = r##"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>RLEval — Replay Analysis</title>
<style>
  :root {
    --bg: #0e1116; --panel: #161b22; --panel2: #1c232d; --line: #2b333d;
    --fg: #e6edf3; --muted: #8b949e; --accent: #4493f8; --blue: #4493f8;
    --orange: #f0883e; --good: #3fb950; --warn: #d29922;
  }
  * { box-sizing: border-box; }
  body { margin: 0; background: var(--bg); color: var(--fg);
    font: 14px/1.5 system-ui, -apple-system, Segoe UI, Roboto, sans-serif; }
  header { display: flex; align-items: center; gap: 16px; padding: 12px 20px;
    background: var(--panel); border-bottom: 1px solid var(--line); }
  header h1 { font-size: 17px; margin: 0; letter-spacing: .3px; }
  header h1 small { color: var(--muted); font-weight: 400; font-size: 12px; }
  .wrap { max-width: 1200px; margin: 0 auto; padding: 20px; }
  .drop { border: 2px dashed var(--line); border-radius: 10px; padding: 32px;
    text-align: center; color: var(--muted); transition: .15s; cursor: pointer; }
  .drop.hot { border-color: var(--accent); color: var(--fg); background: var(--panel); }
  .samples { margin-top: 14px; display: flex; gap: 8px; flex-wrap: wrap; align-items: center; }
  .samples span { color: var(--muted); }
  button.s { background: var(--panel2); color: var(--fg); border: 1px solid var(--line);
    border-radius: 6px; padding: 6px 12px; cursor: pointer; font-size: 13px; }
  button.s:hover { border-color: var(--accent); }
  .status { margin-top: 14px; color: var(--muted); min-height: 20px; }
  .status.err { color: #f85149; }
  .spinner { display: inline-block; width: 14px; height: 14px; border: 2px solid var(--line);
    border-top-color: var(--accent); border-radius: 50%; animation: spin .7s linear infinite;
    vertical-align: -2px; margin-right: 8px; }
  @keyframes spin { to { transform: rotate(360deg); } }
  #summary { display: none; margin-top: 18px; }
  .meta { display: flex; gap: 20px; flex-wrap: wrap; align-items: baseline;
    padding: 12px 16px; background: var(--panel); border: 1px solid var(--line);
    border-radius: 8px; }
  .meta b { font-size: 16px; }
  .meta .k { color: var(--muted); font-size: 12px; text-transform: uppercase; letter-spacing: .5px; }
  .score { font-variant-numeric: tabular-nums; }
  .badge { padding: 1px 7px; border-radius: 10px; font-size: 12px; background: var(--panel2); }
  .badge.t0 { color: #79c0ff; } .badge.t1 { color: var(--orange); }
  .nonstd { margin-top: 12px; padding: 10px 14px; border-radius: 8px;
    background: #3a2d12; color: var(--warn); border: 1px solid #5c4612; }
  nav.tabs { display: flex; gap: 4px; margin-top: 18px; border-bottom: 1px solid var(--line); }
  nav.tabs button { background: none; border: none; color: var(--muted); cursor: pointer;
    padding: 10px 16px; font-size: 14px; border-bottom: 2px solid transparent; }
  nav.tabs button.active { color: var(--fg); border-bottom-color: var(--accent); }
  .tab { display: none; padding-top: 16px; }
  .tab.active { display: block; }
  iframe { width: 100%; border: 1px solid var(--line); border-radius: 8px; background: #fff; }
  iframe.viewer { height: 78vh; }
  iframe.report { height: 80vh; background: #fff; }
  table { width: 100%; border-collapse: collapse; font-variant-numeric: tabular-nums; }
  th, td { text-align: left; padding: 8px 10px; border-bottom: 1px solid var(--line); }
  th { color: var(--muted); font-weight: 600; font-size: 12px; text-transform: uppercase; letter-spacing: .4px; }
  td.num, th.num { text-align: right; }
  tr.t0 td:first-child { box-shadow: inset 3px 0 var(--blue); }
  tr.t1 td:first-child { box-shadow: inset 3px 0 var(--orange); }
  .pos { color: var(--good); } .neg { color: #f85149; }
  .muted { color: var(--muted); }
  .hint { color: var(--muted); font-size: 12px; margin: 4px 0 12px; }
</style>
</head>
<body>
<header>
  <h1>RLEval <small>· unified replay analysis</small></h1>
  <span class="muted" id="ver"></span>
</header>
<div class="wrap">
  <div id="loader">
    <div class="drop" id="drop">
      <strong>Drop a .replay here</strong> or click to choose a file
      <input type="file" id="file" accept=".replay" hidden>
    </div>
    <div class="samples" id="samples"><span>Samples:</span></div>
    <div class="status" id="status"></div>
  </div>

  <div id="summary">
    <div class="meta" id="meta"></div>
    <div class="nonstd" id="nonstd" style="display:none">
      Non-standard map — positional metrics assume standard Soccar, so scores are flagged low-confidence.
    </div>
    <nav class="tabs" id="tabs">
      <button data-tab="overview" class="active">Overview</button>
      <button data-tab="viewer">3D Viewer</button>
      <button data-tab="scoring">Scoring</button>
      <button data-tab="skills">Skills</button>
      <button data-tab="impact">Impact</button>
    </nav>
    <div class="tab active" id="tab-overview"></div>
    <div class="tab" id="tab-viewer"></div>
    <div class="tab" id="tab-scoring"></div>
    <div class="tab" id="tab-skills"></div>
    <div class="tab" id="tab-impact"></div>
  </div>
</div>

<script>
const $ = (id) => document.getElementById(id);
const statusEl = $("status");
let DATA = null;

function setStatus(msg, isErr) {
  statusEl.className = "status" + (isErr ? " err" : "");
  statusEl.innerHTML = msg;
}
function fmt(x, d = 1) { return (x == null || isNaN(x)) ? "—" : Number(x).toFixed(d); }
function teamName(t) { return t === 0 ? "Blue" : t === 1 ? "Orange" : "—"; }
function esc(s) { return String(s).replace(/[&<>"]/g, c => ({ "&":"&amp;","<":"&lt;",">":"&gt;","\"":"&quot;" }[c])); }

// ---- load + analyze ----
async function loadSamples() {
  try {
    const r = await fetch("/api/samples");
    const { samples } = await r.json();
    const box = $("samples");
    samples.forEach(name => {
      const b = document.createElement("button");
      b.className = "s"; b.textContent = name;
      b.onclick = () => analyzeSample(name);
      box.appendChild(b);
    });
    if (!samples.length) box.style.display = "none";
  } catch (e) { /* samples are optional */ }
}

async function analyzeSample(name) {
  busy(`Analyzing sample <b>${esc(name)}</b>…`);
  await run(fetch("/api/analyze/sample/" + encodeURIComponent(name)));
}
async function analyzeFile(file) {
  busy(`Analyzing <b>${esc(file.name)}</b> (${(file.size/1e6).toFixed(1)} MB)…`);
  const buf = await file.arrayBuffer();
  await run(fetch("/api/analyze?name=" + encodeURIComponent(file.name), {
    method: "POST", headers: { "Content-Type": "application/octet-stream" }, body: buf,
  }));
}
function busy(msg) { setStatus(`<span class="spinner"></span>${msg}`); }

async function run(promise) {
  const t0 = performance.now();
  try {
    const resp = await promise;
    if (!resp.ok) throw new Error(await resp.text());
    DATA = await resp.json();
    render();
    const ms = Math.round(performance.now() - t0);
    setStatus(`Done in ${(ms/1000).toFixed(1)}s. Drop another replay to re-analyze.`);
  } catch (e) {
    setStatus("Error: " + esc(e.message || e), true);
  }
}

// ---- rendering ----
function render() {
  const d = DATA;
  $("summary").style.display = "block";
  const score = Object.entries(d.team_scores || {}).length
    ? d.team_scores.map(([t, s]) => `${teamName(t)} ${s}`).join(" – ")
    : (d.team_scores || []).map(p => `${teamName(p[0])} ${p[1]}`).join(" – ");
  $("meta").innerHTML = `
    <div><div class="k">Replay</div><b>${esc(d.replay_id)}</b></div>
    <div><div class="k">Map</div><b>${esc(d.map || "—")}</b></div>
    <div><div class="k">Mode</div><b>${d.team_size ? d.team_size + "v" + d.team_size : "—"}</b></div>
    <div><div class="k">Duration</div><b>${fmt(d.duration_s, 0)}s</b></div>
    <div><div class="k">Score</div><b class="score">${score || "—"}</b></div>`;
  $("nonstd").style.display = d.standard_map ? "none" : "block";

  renderOverview(d);
  renderSkills(d);
  renderImpact(d);
  // The heavy iframes are filled lazily on first tab open.
  viewerLoaded = scoringLoaded = false;
  selectTab("overview");
}

function renderOverview(d) {
  const impactByPri = {};
  (d.impact.players || []).forEach(p => impactByPri[p.pri] = p);
  const skillByPri = {};
  (d.skill_profiles || []).forEach(p => skillByPri[p.pri] = p);
  const rows = (d.scores || []).slice().sort((a, b) => b.composite - a.composite).map(r => {
    const imp = impactByPri[r.target_pri];
    const sk = skillByPri[r.target_pri];
    const dv = imp ? imp.sum_dv : null;
    return `<tr class="t${r.target_team}">
      <td>${esc(r.target_player)}</td>
      <td><span class="badge t${r.target_team}">${teamName(r.target_team)}</span></td>
      <td class="num">${fmt(r.composite)}</td>
      <td>${esc(r.licence)}</td>
      <td>${esc(r.player_type)}</td>
      <td class="num">${sk ? fmt(sk.total_per_min, 1) : "—"}</td>
      <td class="num ${dv >= 0 ? "pos" : "neg"}">${dv == null ? "—" : (dv >= 0 ? "+" : "") + fmt(dv, 3)}</td>
      <td class="muted">${esc(r.main_leak)}</td>
    </tr>`;
  }).join("");
  $("tab-overview").innerHTML = `
    <p class="hint">One row per player — decision-discipline composite (scoring), mechanical activity (skills/min),
       and value impact (ΔV). Open the tabs for the full 3D replay, the scoring report, and per-skill detail.</p>
    <table>
      <thead><tr>
        <th>Player</th><th>Team</th><th class="num">Composite</th><th>Licence</th>
        <th>Type</th><th class="num">Skills/min</th><th class="num">Impact ΔV</th><th>Main leak</th>
      </tr></thead>
      <tbody>${rows || `<tr><td colspan="8" class="muted">No players scored.</td></tr>`}</tbody>
    </table>`;
}

function renderSkills(d) {
  // Union of all skills present, catalog-ordered as they appear per player.
  const cols = [];
  (d.skill_profiles || []).forEach(p => Object.keys(p.skills).forEach(k => { if (!cols.includes(k)) cols.push(k); }));
  const head = cols.map(c => `<th class="num">${esc(c)}</th>`).join("");
  const rows = (d.skill_profiles || []).map(p => {
    const cells = cols.map(c => {
      const st = p.skills[c];
      return `<td class="num">${st ? st.count : "·"}</td>`;
    }).join("");
    return `<tr class="t${p.team}">
      <td>${esc(p.player)}</td>
      <td><span class="badge t${p.team}">${teamName(p.team)}</span></td>
      <td class="num">${fmt(p.total_per_min, 1)}</td>
      ${cells}
    </tr>`;
  }).join("");
  $("tab-skills").innerHTML = `
    <p class="hint">Mechanical skills detected from kinematics (counts per player). Heuristic — thresholds are versioned.</p>
    <table>
      <thead><tr><th>Player</th><th>Team</th><th class="num">Total/min</th>${head}</tr></thead>
      <tbody>${rows || `<tr><td class="muted">No skills detected.</td></tr>`}</tbody>
    </table>`;
}

function renderImpact(d) {
  const rows = (d.impact.players || []).map(p => `
    <tr class="t${p.team}">
      <td>${esc(p.player || "—")}</td>
      <td><span class="badge t${p.team}">${teamName(p.team)}</span></td>
      <td class="num">${p.touches}</td>
      <td class="num ${p.sum_dv >= 0 ? "pos" : "neg"}">${(p.sum_dv >= 0 ? "+" : "") + fmt(p.sum_dv, 3)}</td>
      <td class="num ${p.mean_dv >= 0 ? "pos" : "neg"}">${(p.mean_dv >= 0 ? "+" : "") + fmt(p.mean_dv, 4)}</td>
    </tr>`).join("");
  $("tab-impact").innerHTML = `
    <p class="hint">Value model (ΔV): each player's summed per-touch swing in P(their team scores next).
       Trained in-process on this match; an independent cross-check on the scoring rubric.
       Base rate ${fmt(d.impact.base_rate, 3)}, log-loss ${fmt(d.impact.log_loss, 3)}.</p>
    <table>
      <thead><tr><th>Player</th><th>Team</th><th class="num">Touches</th>
        <th class="num">Total ΔV</th><th class="num">Mean ΔV</th></tr></thead>
      <tbody>${rows || `<tr><td class="muted">No value data.</td></tr>`}</tbody>
    </table>`;
}

// ---- tabs (lazy iframes for the heavy HTML views) ----
let viewerLoaded = false, scoringLoaded = false;
function selectTab(name) {
  document.querySelectorAll("nav.tabs button").forEach(b =>
    b.classList.toggle("active", b.dataset.tab === name));
  document.querySelectorAll(".tab").forEach(t =>
    t.classList.toggle("active", t.id === "tab-" + name));
  if (name === "viewer" && !viewerLoaded) {
    $("tab-viewer").innerHTML = `<iframe class="viewer" srcdoc="${esc(DATA.viewer_html)}"></iframe>`;
    viewerLoaded = true;
  }
  if (name === "scoring" && !scoringLoaded) {
    $("tab-scoring").innerHTML = `<iframe class="report" srcdoc="${esc(DATA.scoring_html)}"></iframe>`;
    scoringLoaded = true;
  }
}
document.querySelectorAll("nav.tabs button").forEach(b =>
  b.onclick = () => selectTab(b.dataset.tab));

// ---- drop zone wiring ----
const drop = $("drop"), fileInput = $("file");
drop.onclick = () => fileInput.click();
fileInput.onchange = () => { if (fileInput.files[0]) analyzeFile(fileInput.files[0]); };
["dragenter", "dragover"].forEach(ev => drop.addEventListener(ev, e => {
  e.preventDefault(); drop.classList.add("hot");
}));
["dragleave", "drop"].forEach(ev => drop.addEventListener(ev, e => {
  e.preventDefault(); drop.classList.remove("hot");
}));
drop.addEventListener("drop", e => {
  const f = e.dataTransfer.files[0];
  if (f) analyzeFile(f);
});

loadSamples();
</script>
</body>
</html>
"##;
