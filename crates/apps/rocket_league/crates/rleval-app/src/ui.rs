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
    --bg: #0b0e14; --bg-soft: #11151d; --card: #141a23; --card-2: #1a212c;
    --line: #243040; --line-soft: #1b2330;
    --fg: #e8eef6; --muted: #93a1b5; --faint: #687586;
    --accent: #5b9dff; --accent-2: #8a6bff;
    --blue: #4f9dfd; --orange: #ff9f45;
    --good: #46d18b; --bad: #ff6b6b; --warn: #e3b341;
    --radius: 14px;
    --shadow: 0 1px 0 rgba(255,255,255,.03), 0 10px 30px -16px rgba(0,0,0,.7);
    --mono: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  }
  * { box-sizing: border-box; }
  html, body { height: 100%; }
  body {
    margin: 0; color: var(--fg);
    font: 14px/1.55 system-ui, -apple-system, "Segoe UI", Roboto, sans-serif;
    background:
      radial-gradient(1100px 560px at 12% -12%, rgba(138,107,255,.12), transparent 60%),
      radial-gradient(1000px 520px at 102% -4%, rgba(91,157,255,.12), transparent 56%),
      var(--bg);
    background-attachment: fixed;
  }
  a { color: var(--accent); }

  /* ---- header ---- */
  header { position: sticky; top: 0; z-index: 30;
    background: rgba(11,14,20,.72); backdrop-filter: blur(10px);
    border-bottom: 1px solid var(--line-soft); }
  .hd { max-width: 1180px; margin: 0 auto; display: flex; align-items: center;
    gap: 13px; padding: 13px 22px; }
  .logo { width: 31px; height: 31px; border-radius: 9px; display: grid; place-items: center;
    font-weight: 800; font-size: 12px; color: #fff; letter-spacing: .5px;
    background: linear-gradient(135deg, var(--accent), var(--accent-2));
    box-shadow: 0 6px 18px -6px var(--accent); }
  .brand { font-weight: 700; font-size: 16px; letter-spacing: .2px; }
  .brand small { color: var(--muted); font-weight: 500; font-size: 12px; margin-left: 7px; }
  .hd .spacer { flex: 1; }
  .pill-id { display: none; font: 12px var(--mono); color: var(--muted);
    background: var(--card); border: 1px solid var(--line); padding: 5px 11px; border-radius: 999px; }

  .wrap { max-width: 1180px; margin: 0 auto; padding: 26px 22px 64px; }

  /* ---- hero / dropzone ---- */
  .hero { background: linear-gradient(180deg, var(--card), var(--bg-soft));
    border: 1px solid var(--line); border-radius: var(--radius); padding: 26px;
    box-shadow: var(--shadow); transition: padding .2s; }
  .drop { border: 1.5px dashed var(--line); border-radius: 12px; padding: 34px 20px;
    text-align: center; cursor: pointer; transition: .18s; background: rgba(255,255,255,.012); }
  .drop:hover { border-color: var(--accent); background: rgba(91,157,255,.04); }
  .drop.hot { border-color: var(--accent); background: rgba(91,157,255,.08); transform: translateY(-1px); }
  .drop .ic { width: 44px; height: 44px; margin: 0 auto 12px; color: var(--accent); display: block; }
  .drop h2 { margin: 0 0 5px; font-size: 17px; font-weight: 700; }
  .drop p { margin: 0; color: var(--muted); font-size: 13px; }
  .samples { margin-top: 16px; display: flex; gap: 8px; flex-wrap: wrap; align-items: center; }
  .samples .lbl { color: var(--faint); font-size: 11px; text-transform: uppercase; letter-spacing: .7px; }
  .chip { background: var(--card-2); color: var(--fg); border: 1px solid var(--line);
    border-radius: 999px; padding: 6px 14px; font-size: 13px; cursor: pointer; transition: .15s; }
  .chip:hover { border-color: var(--accent); color: #fff; background: rgba(91,157,255,.1); }
  .status { margin-top: 15px; min-height: 20px; font-size: 13px; color: var(--muted);
    display: flex; align-items: center; gap: 9px; }
  .status.err { color: var(--bad); }
  .spinner { display: inline-block; width: 15px; height: 15px; border: 2px solid var(--line);
    border-top-color: var(--accent); border-radius: 50%; animation: spin .7s linear infinite; }
  @keyframes spin { to { transform: rotate(360deg); } }
  /* once a result is on screen, the hero shrinks to a slim re-analyze bar */
  body.has-result .hero { padding: 14px 18px; }
  body.has-result .drop { padding: 14px 16px; }
  body.has-result .drop .ic, body.has-result .drop p { display: none; }
  body.has-result .drop h2 { font-size: 14px; color: var(--muted); font-weight: 600; }

  /* ---- summary ---- */
  #summary { display: none; margin-top: 22px; }
  @keyframes rise { from { opacity: 0; transform: translateY(8px); } to { opacity: 1; transform: none; } }
  .stats { display: grid; grid-template-columns: repeat(auto-fit, minmax(140px, 1fr)); gap: 12px; }
  .stat { background: var(--card); border: 1px solid var(--line); border-radius: 12px;
    padding: 13px 16px; box-shadow: var(--shadow); }
  .stat .k { color: var(--faint); font-size: 11px; text-transform: uppercase; letter-spacing: .7px; }
  .stat .v { font-size: 19px; font-weight: 700; margin-top: 5px; }
  .scoreboard { display: flex; align-items: center; gap: 9px; margin-top: 3px; font-weight: 800; font-size: 19px; }
  .sb-t0 { color: var(--blue); } .sb-t1 { color: var(--orange); }
  .sbsep { color: var(--faint); font-weight: 500; }
  .nonstd { margin-top: 14px; display: flex; gap: 10px; align-items: center; padding: 12px 16px;
    border-radius: 12px; font-size: 13px; color: var(--warn);
    background: linear-gradient(90deg, rgba(227,179,65,.13), transparent);
    border: 1px solid rgba(227,179,65,.35); }
  .nonstd svg { width: 18px; height: 18px; flex: none; }

  /* ---- tabs ---- */
  nav.tabs { display: inline-flex; gap: 4px; margin: 22px 0 0; padding: 5px;
    background: var(--card); border: 1px solid var(--line); border-radius: 12px; box-shadow: var(--shadow); }
  nav.tabs button { border: 0; background: transparent; color: var(--muted); cursor: pointer;
    padding: 8px 16px; font-size: 13.5px; font-weight: 600; border-radius: 8px; transition: .15s; }
  nav.tabs button:hover { color: var(--fg); }
  nav.tabs button.active { color: #fff;
    background: linear-gradient(135deg, var(--accent), var(--accent-2));
    box-shadow: 0 8px 18px -10px var(--accent); }
  .tab { display: none; }
  .tab.active { display: block; margin-top: 18px; animation: rise .26s ease both; }

  /* ---- cards + tables ---- */
  .card { background: var(--card); border: 1px solid var(--line); border-radius: var(--radius);
    box-shadow: var(--shadow); overflow: hidden; }
  .hint { padding: 13px 18px; margin: 0; color: var(--muted); font-size: 12.5px;
    border-bottom: 1px solid var(--line-soft); background: var(--bg-soft); }
  .tablewrap { overflow: auto; }
  table { width: 100%; border-collapse: collapse; font-size: 13.5px; }
  thead th { position: sticky; top: 0; background: var(--card-2); color: var(--faint);
    font-weight: 600; font-size: 11px; text-transform: uppercase; letter-spacing: .5px;
    text-align: left; padding: 11px 14px; border-bottom: 1px solid var(--line); white-space: nowrap; }
  tbody td { padding: 11px 14px; border-bottom: 1px solid var(--line-soft); vertical-align: middle; }
  tbody tr:last-child td { border-bottom: 0; }
  tbody tr:hover { background: rgba(255,255,255,.025); }
  td.num, th.num { text-align: right; font-variant-numeric: tabular-nums; font-family: var(--mono); }
  .pl { display: flex; align-items: center; gap: 9px; }
  .dot { width: 8px; height: 8px; border-radius: 50%; flex: none; }
  .dot.t0 { background: var(--blue); box-shadow: 0 0 0 3px rgba(79,157,253,.15); }
  .dot.t1 { background: var(--orange); box-shadow: 0 0 0 3px rgba(255,159,69,.15); }
  .nm { font-weight: 600; }
  .badge { font-size: 11px; padding: 2px 9px; border-radius: 999px; font-weight: 600; border: 1px solid transparent; }
  .badge.t0 { color: var(--blue); background: rgba(79,157,253,.12); border-color: rgba(79,157,253,.25); }
  .badge.t1 { color: var(--orange); background: rgba(255,159,69,.12); border-color: rgba(255,159,69,.25); }
  .big { font-weight: 700; font-size: 14px; }
  .meter { position: relative; height: 6px; border-radius: 999px; background: var(--line);
    overflow: hidden; margin-top: 5px; min-width: 70px; }
  .meter > i { position: absolute; left: 0; top: 0; bottom: 0; border-radius: 999px;
    background: linear-gradient(90deg, var(--accent), var(--accent-2)); }
  .dv { display: inline-flex; align-items: center; gap: 9px; justify-content: flex-end; }
  .dvbar { position: relative; width: 84px; height: 8px; background: var(--line); border-radius: 999px; flex: none; }
  .dvbar::before { content: ""; position: absolute; left: 50%; top: -2px; bottom: -2px;
    width: 1px; background: var(--faint); opacity: .5; }
  .dvbar > i { position: absolute; top: 0; bottom: 0; border-radius: 999px; }
  .kc { display: inline-block; min-width: 26px; text-align: center; padding: 2px 8px; border-radius: 7px;
    font-variant-numeric: tabular-nums; font-family: var(--mono); font-size: 12.5px; }
  .kc.on { background: rgba(91,157,255,.14); color: #cfe2ff; border: 1px solid rgba(91,157,255,.25); }
  .kc.off { color: var(--faint); }
  .pos { color: var(--good); } .neg { color: var(--bad); }
  .muted { color: var(--muted); }

  /* ---- Improve tab: per-player recommendation cards ---- */
  .reco { padding: 13px 18px; border-bottom: 1px solid var(--line-soft); border-left: 3px solid var(--line); }
  .reco:last-child { border-bottom: 0; }
  .reco-major { border-left-color: var(--bad); }
  .reco-leak { border-left-color: var(--warn); }
  .reco-secondary { border-left-color: var(--accent); }
  .reco-note { border-left-color: var(--line); }
  .reco-strength { border-left-color: var(--good); }
  .reco-h { display: flex; align-items: baseline; gap: 8px; margin-bottom: 4px; flex-wrap: wrap; }
  .reco-tag { font-size: 10px; text-transform: uppercase; letter-spacing: .6px; color: var(--faint); font-weight: 700; }
  .reco-tip { color: var(--muted); font-size: 13px; line-height: 1.5; }

  /* ---- History panel: cross-match habits ---- */
  #history { display: none; margin-top: 22px; }
  #history.open { display: block; animation: rise .26s ease both; }
  .hist-bar { display: flex; gap: 10px; align-items: center; flex-wrap: wrap; margin-bottom: 14px; }
  .hist-bar select, .hist-bar input { background: var(--card-2); color: var(--fg); border: 1px solid var(--line);
    border-radius: 9px; padding: 8px 12px; font-size: 13px; }
  .focus { border-left: 3px solid var(--accent); }
  .focus .reco-tag { color: var(--accent); }
  .gapneg { color: var(--bad); } .gappos { color: var(--good); }
  .trend { display: flex; gap: 4px; align-items: flex-end; height: 54px; padding: 12px 18px; }
  .trend i { flex: 1; min-width: 6px; max-width: 26px; border-radius: 3px 3px 0 0; background: var(--faint); }
  .trend i.w { background: var(--good); } .trend i.l { background: var(--bad); }

  .mrow { cursor: pointer; }
  .mrow:hover td { background: var(--card-2); }

  /* ---- iframes (viewer / report) ---- */
  .vtoolbar { display: flex; gap: 12px; align-items: center; margin-bottom: 12px; }
  .btn { display: inline-flex; align-items: center; gap: 8px; background: var(--card-2); color: var(--fg);
    border: 1px solid var(--line); border-radius: 9px; padding: 8px 14px; font-size: 13px; font-weight: 600;
    cursor: pointer; transition: .15s; }
  .btn:hover { border-color: var(--accent); color: #fff; background: rgba(91,157,255,.1); }
  iframe { width: 100%; border: 1px solid var(--line); border-radius: 12px; background: #fff; display: block; }
  iframe.viewer { height: 78vh; }
  iframe.report { height: 82vh; }
  iframe.viewer:fullscreen, iframe.viewer:-webkit-full-screen {
    width: 100vw; height: 100vh; border: 0; border-radius: 0; }
</style>
</head>
<body>
<header>
  <div class="hd">
    <div class="logo">RL</div>
    <div class="brand">RLEval <small>unified replay analysis</small></div>
    <div class="spacer"></div>
 <span id="who" style="display:none;font-size:13px;margin-right:14px"></span>
    <a href="/auth/login" id="signIn" style="display:none;font-size:13px;font-weight:600;margin-right:14px">Sign in with Google</a>
    <a href="#" id="signOut" style="display:none;font-size:13px;font-weight:600;margin-right:14px">Sign out</a>
    <a href="#" id="histLink" style="font-size:13px;font-weight:600;margin-right:14px">History</a>
    <a href="/admin" style="font-size:13px;font-weight:600;margin-right:14px">Admin</a>
    <span class="pill-id" id="hdId"></span>
  </div>
</header>
<div class="wrap">
  <div id="loader">
    <div class="hero">
      <div class="drop" id="drop">
        <svg class="ic" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6"
             stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <path d="M12 15V3"/><path d="M8 7l4-4 4 4"/>
          <path d="M4 15v3a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-3"/>
        </svg>
        <h2>Drop a .replay to analyze</h2>
        <p>or click to browse — the file is parsed locally, nothing leaves your machine</p>
        <input type="file" id="file" accept=".replay" hidden>
      </div>
      <div class="samples" id="samples"><span class="lbl">Samples</span></div>
      <div class="samples"><span class="lbl">Rank</span>
        <select id="uploadRank"><option value="">auto (from the lobby)</option><option>silver</option><option>gold</option><option>platinum</option><option>diamond</option><option>champion</option><option>grand-champion</option></select></div>
      <div class="samples" id="teamPick" style="display:none"><span class="lbl">Share with team</span>
        <select id="uploadTeam"><option value="">just me</option></select></div>
      <div class="status" id="status"></div>
    </div>
  </div>

  <div id="history">
    <div class="hist-bar">
      <input type="password" id="tokenInput" placeholder="Access token (only if the server requires one)" size="34" autocomplete="off">
      <button class="btn" id="tokenSave">Use token</button>
    </div>
    <div id="teamBody"></div>
    <div id="historyBody"></div>
  </div>

  <div id="summary">
    <div class="stats" id="meta"></div>
    <div class="nonstd" id="nonstd" style="display:none">
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"
           stroke-linecap="round" stroke-linejoin="round"><path d="M12 9v4M12 17h.01"/>
        <path d="M10.3 3.9 1.8 18a2 2 0 0 0 1.7 3h17a2 2 0 0 0 1.7-3L13.7 3.9a2 2 0 0 0-3.4 0Z"/></svg>
      Non-standard map — positional metrics assume standard Soccar, so scores are flagged low-confidence.
    </div>
    <div class="nonstd" id="coordwarn" style="display:none">
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"
           stroke-linecap="round" stroke-linejoin="round"><path d="M12 9v4M12 17h.01"/>
        <path d="M10.3 3.9 1.8 18a2 2 0 0 0 1.7 3h17a2 2 0 0 0 1.7-3L13.7 3.9a2 2 0 0 0-3.4 0Z"/></svg>
      <span id="coordwarnText"></span>
    </div>
    <nav class="tabs" id="tabs">
      <button data-tab="overview" class="active">Overview</button>
      <button data-tab="improve">Improve</button>
      <button data-tab="moments">Moments</button>
      <button data-tab="stats">Stats</button>
      <button data-tab="viewer">3D Viewer</button>
      <button data-tab="scoring">Scoring</button>
      <button data-tab="skills">Skills</button>
      <button data-tab="impact">Impact</button>
      <button data-tab="pacifist">Pacifist</button>
      <button data-tab="ballchasing">Ballchasing</button>
    </nav>
    <div class="tab active" id="tab-overview"></div>
    <div class="tab" id="tab-improve"></div>
    <div class="tab" id="tab-moments"></div>
    <div class="tab" id="tab-stats"></div>
    <div class="tab" id="tab-viewer"></div>
    <div class="tab" id="tab-scoring"></div>
    <div class="tab" id="tab-skills"></div>
    <div class="tab" id="tab-impact"></div>
    <div class="tab" id="tab-pacifist"></div>
    <div class="tab" id="tab-ballchasing"></div>
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
function clampPct(x) { return Math.max(0, Math.min(100, Number(x) || 0)); }
function signed(v, d) { return (v >= 0 ? "+" : "") + fmt(v, d); }

// ---- load + analyze ----
async function loadSamples() {
  try {
    const r = await fetch("/api/samples");
    const { samples } = await r.json();
    const box = $("samples");
    samples.forEach(name => {
      const b = document.createElement("button");
      b.className = "chip"; b.textContent = name;
      b.onclick = () => analyzeSample(name);
      box.appendChild(b);
    });
    if (!samples.length) box.style.display = "none";
  } catch (e) { /* samples are optional */ }
}

async function analyzeSample(name) {
  busy(`Analyzing sample <b>${esc(name)}</b>…`);
  await run(authFetch("/api/analyze/sample/" + encodeURIComponent(name) + teamParam("?")));
}
async function analyzeFile(file) {
  busy(`Analyzing <b>${esc(file.name)}</b> (${(file.size/1e6).toFixed(1)} MB)…`);
  const buf = await file.arrayBuffer();
  await run(authFetch("/api/analyze?name=" + encodeURIComponent(file.name) + teamParam("&"), {
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
    setStatus(`Done in ${(ms/1000).toFixed(1)}s · drop another replay to re-analyze.`);
  } catch (e) {
    setStatus("Error: " + esc(e.message || e), true);
  }
}

// ---- rendering ----
function render() {
  const d = DATA;
  document.body.classList.add("has-result");
  $("summary").style.display = "block";
  $("summary").style.animation = "rise .35s ease both";

  const hd = $("hdId");
  if (hd) { hd.textContent = d.replay_id; hd.style.display = "inline-block"; }

  const pairs = (d.team_scores || []).map(p => Array.isArray(p) ? p : [p[0], p[1]]);
  const scoreboard = pairs.length
    ? pairs.map(([t, s]) => `<span class="sb-t${t}">${s}</span>`).join('<span class="sbsep">–</span>')
    : "—";
  const stat = (k, v) => `<div class="stat"><div class="k">${k}</div><div class="v">${v}</div></div>`;
  $("meta").innerHTML =
    stat("Map", esc(d.map || "—")) +
    stat("Mode", d.team_size ? d.team_size + "v" + d.team_size : "—") +
    stat("Duration", fmt(d.duration_s, 0) + "s") +
    stat("Players", (d.scores || []).length) +
    `<div class="stat"><div class="k">Score</div><div class="scoreboard">${scoreboard}</div></div>`;
  $("nonstd").style.display = d.standard_map ? "none" : "flex";
  const cw = d.coordinate_warnings || [];
  $("coordwarn").style.display = cw.length ? "flex" : "none";
  $("coordwarnText").innerHTML = cw.map(esc).join("<br>");

  renderOverview(d);
  renderImprove(d);
  renderMoments(d);
  renderStats(d);
  renderSkills(d);
  renderImpact(d);
  renderPacifist(d);
  // The heavy iframes are filled lazily on first tab open.
  for (const k in panelLoaded) delete panelLoaded[k];
  $("tab-viewer").innerHTML = $("tab-scoring").innerHTML = "";
  selectTab("overview");
}

function nameCell(team, name) {
  return `<div class="pl"><span class="dot t${team}"></span><span class="nm">${esc(name)}</span></div>`;
}
function cardTable(hint, head, body, empty, cols) {
  return `<div class="card"><p class="hint">${hint}</p><div class="tablewrap"><table>
    <thead><tr>${head}</tr></thead>
    <tbody>${body || `<tr><td colspan="${cols}" class="muted">${empty}</td></tr>`}</tbody>
  </table></div></div>`;
}

// Per player: each metric against the bracket median and the next bracket's median.
function rankGaps(d) {
  const cards = (d.scores || []).filter(r => r.relative).map(r => {
    const rows = r.relative.metrics.filter(m => m.raw != null).map(m => `<tr>
      <td>${esc(m.key.replaceAll("_", " "))}</td><td class="num">${fmt(m.raw, 2)}</td>
      <td class="num">${fmt(m.within_rank_pct, 0)}</td><td class="num">${fmt(m.bracket_median, 2)}</td>
      <td class="num">${m.next_median == null ? "—" : fmt(m.next_median, 2)}</td>
      <td class="num">${m.next_median == null ? "—" : signed(m.next_median - m.raw, 2)}</td></tr>`).join("");
    return `<details class="card"><summary>${nameCell(r.target_team, r.target_player)} — vs ${esc(r.relative.bracket)}</summary>
      <div class="tablewrap"><table><thead><tr><th>Metric</th><th class="num">You</th><th class="num">Peer pct</th>
      <th class="num">${esc(r.relative.bracket)} median</th><th class="num">Next bracket median</th><th class="num">To next</th></tr></thead>
      <tbody>${rows}</tbody></table></div></details>`;
  }).join("");
  return cards ? `<p class="hint">Where each metric sits against your bracket, and the median of the bracket above it. ` +
    `“To next” is a raw difference — for lower-is-better metrics a negative number is the way up.</p>` + cards : "";
}

function renderOverview(d) {
  const impactByPri = {}; (d.impact.players || []).forEach(p => impactByPri[p.pri] = p);
  const skillByPri = {}; (d.skill_profiles || []).forEach(p => skillByPri[p.pri] = p);
  // The rank-relative layer (if a norms artifact was applied) is lobby-wide:
  // every report carries the same bracket. Use the first to label the column.
  const rel0 = (d.scores || []).map(r => r.relative).find(Boolean);
  const rows = (d.scores || []).slice().sort((a, b) => b.composite - a.composite).map(r => {
    const imp = impactByPri[r.target_pri];
    const sk = skillByPri[r.target_pri];
    const dv = imp ? imp.sum_dv : null;
    const rv = r.relative;
    // "vs rank": where this player's composite sits within their bracket (0–100).
    const relCell = rv
      ? `<td class="num"><div class="big">${fmt(rv.composite_pct, 0)}<span class="muted" style="font-size:11px">&nbsp;pct</span></div>
          <div class="meter"><i style="width:${clampPct(rv.composite_pct)}%"></i></div></td>`
      : (rel0 ? `<td class="num muted">—</td>` : "");
    // Main (absolute) leak, plus the rank-relative leak when it carries signal.
    const relLeak = rv && rv.rank_relative_leak && rv.rank_relative_leak !== "none"
      ? `<div class="muted" style="font-size:11px">vs rank: ${esc(rv.rank_relative_leak)}</div>` : "";
    return `<tr>
      <td>${nameCell(r.target_team, r.target_player)}</td>
      <td><span class="badge t${r.target_team}">${teamName(r.target_team)}</span></td>
      <td class="num"><div class="big">${fmt(r.composite)}</div>
        <div class="meter"><i style="width:${clampPct(r.composite)}%"></i></div></td>
      <td>${esc(r.licence)}</td>
      ${relCell}
      <td>${esc(r.player_type)}</td>
      <td class="num">${sk ? fmt(sk.total_per_min, 1) : "—"}</td>
      <td class="num ${dv >= 0 ? "pos" : "neg"}">${dv == null ? "—" : signed(dv, 3)}</td>
      <td class="muted">${esc(r.main_leak)}${relLeak}</td>
    </tr>`;
  }).join("");
  const relHead = rel0 ? `<th class="num">vs rank</th>` : "";
  const head = `<th>Player</th><th>Team</th><th class="num">Composite</th><th>Licence</th>
    ${relHead}<th>Type</th><th class="num">Skills/min</th><th class="num">Impact ΔV</th><th>Main leak</th>`;
  const banner = rel0
    ? `<p class="hint">Rank-relative grading is on: this lobby is graded against <b>${esc(rel0.bracket)}</b> ` +
      `(${esc(rel0.basis)}-inferred, mean tier ${fmt(rel0.bracket_tier_mean, 1)}). The absolute composite/licence ` +
      `is unchanged; <b>vs rank</b> is your percentile within that bracket, and the <b>vs rank</b> leak is where you ` +
      `most lag peers of your own level.</p>`
    : "";
  $("tab-overview").innerHTML = banner + rankGaps(d) + cardTable(
    "One row per player — decision-discipline composite (scoring), mechanical activity (skills/min), and value impact (ΔV). Open the tabs for the full 3D replay, the scoring report, and per-skill detail.",
    head, rows, "No players scored.", rel0 ? 9 : 8);
}

// ---- Improve tab: turn existing signals into a short "what to work on" list.
//
// Deliberately templated, not generated — every tip below is a fixed string
// keyed by an existing metric key or Pacifist fault criterion, the same
// "config over code" spirit as the leak→chapter map already in `replay-scoring`
// (chapter labels here match `scoring::config::ScoreConfig`'s `chapter` field).
// This is the assess-vs-suggest gap: the scoring/Pacifist tabs already compute
// the single worst metric (`main_leak`) and the fault list, but only report
// *that* something is off, not what to actually change.
const METRIC_TIPS = {
  overcommit_rate: ["Controlled counterattacks", "You're crossing the ball/midline as 1st man without keeping possession too often, and your 2nd man isn't covering when you do. Hold shape — don't commit past the ball unless you have it or your partner has rotated to cover."],
  goalside_discipline_1st: ["Core game states / defence", "As 1st man in defence, you're getting caught up-field of the ball too often. Stay goal-side — you can't challenge or recover if the ball beats you to the space behind you."],
  challenge_timing: ["Trigger discipline", "Your 50/50 challenges are arriving late, low on boost, or out of control too often. Arrive with boost, on the ground, and on time — a rushed challenge loses the ball as often as it wins it."],
  first_touch_value: ["Ground control / stop booming", "Your first touches as 1st man trend toward blind clears (\"booms\") rather than controlled advances. Take the extra half-second to redirect toward space or a teammate instead of blasting it away."],
  support_spacing: ["Central support", "Your spacing from your teammate drifts outside the healthy support range — either stacking on the same ball (double-commit risk) or too far away to help. The 3D viewer's distance tool shows this live."],
  central_support_fraction: ["Central support", "As 2nd man you're drifting wide or ball-watching instead of holding central, goal-side support. Stay central so you're one rotation away from either the ball or the net."],
  double_commit_rate: ["Central support", "You and your teammate are both pressuring the ball at the same time too often. When your partner commits, hang back — a double commit leaves the net empty if it goes wrong."],
  transition_readiness: ["Core game states / transitions", "When possession flips, you're not consistently positioned to step up into 1st man. Stay goal-side and facing play so a turnover doesn't catch you rotating the wrong way."],
  boost_management: ["Fundamentals / boost", "You're running on empty too often. Route through big pads on your rotations rather than chasing small pads mid-play, and treat 0 boost as an emergency, not a steady state."],
  possession_retention: ["Ground control / stop booming", "Too many of your touches are \"booms\" — high-power clears with no target — instead of keeping possession. Look for a teammate or open space before you commit to power."],
  ball_chase_index: ["Positioning", "You and your teammate are both closing on the ball at once too often — a sign of ball-chasing rather than rotation. One of you should peel off to cover while the other commits."],
  goalside_discipline_team: ["Defence structure", "There are stretches on defence where neither of you is goal-side of the ball. At least one player should always sit between the ball and your net."],
  recovery_speed: ["Air system / recovery", "Your recoveries after aerials/challenges are taking too long. Prioritize getting wheels-down and facing the ball over a stylish landing — every extra second is a second you're out of the play."],
  aerial_presence: ["Air system / aerial threat", "You're rarely leaving the ground. Getting comfortable contesting 50/50s in the air creates a threat your opponents have to respect, not just a defensive option."],
  facing_ball_share: ["Positioning / awareness", "You're facing/staring at the ball a lot of the time — often a sign of ball-watching rather than scanning for rotations and open space. Good positioning means checking around you, not fixating on the ball."],
  reverse_driving: ["Fundamentals / car control", "You're spending a lot of time driving in reverse. Reverse is slower and harder to aim — look for a powerslide turn instead of backing up."],
  boost_starvation: ["Fundamentals / boost", "When you hit 0 boost you're staying stranded there a while before refilling. Path toward a pad as soon as you're empty rather than continuing to play boostless."],
};
// Grounded in the Pacifist guide's own criteria text (docs/pacifist-assessment-criteria.md),
// not invented — FM-2 is the only Major-severity criterion implemented today;
// F4/F9/F17 are Minor (see the rolled-up minor-fault note below).
const FAULT_TIPS = {
  "FM-2": ["Full-speed, empty-tank flip into the corner", "As 2nd man, never flip full-speed into the opponent's corner with 0 boost — the guide's textbook Major mistake. Contain instead of committing when you're empty."],
  "F17": ["Dove in as last man", "When you're the deepest defender with no teammate goal-side behind you, don't dive into a full commit — contain and wait for support instead."],
  "F4": ["Aggressive play on empty boost", "With 0 boost, avoid challenges, dives, or aggressive forward pushes — recover position and collect boost first."],
  "F9": ["Early forward commit as 2nd man", "As 2nd man, hold your support position until the 1st man rotates out or a clear turn arrives, rather than hunting for early involvement."],
};

// Same impact formula scoring's leak selector uses (`effective_weight * (100 −
// normalized)`), but over the runner-up rather than the argmax, so a player
// gets more than one thing to work on. Excludes the primary leak, experimental
// (unpromoted) metrics, and anything already close to a perfect score.
function secondWeakMetric(r) {
  const impact = (m) => m.effective_weight * (100 - m.normalized);
  const candidates = (r.metrics || []).filter(m =>
    !m.experimental && m.raw != null && m.key !== r.main_leak && m.normalized < 85);
  if (!candidates.length) return null;
  return candidates.reduce((a, b) => (impact(b) > impact(a) ? b : a));
}
// The best-performing weighted metric, to close each card on what's working —
// suggesting improvements shouldn't mean only ever pointing out faults.
function bestMetric(r) {
  const good = (r.metrics || []).filter(m =>
    !m.experimental && m.effective_weight > 0 && m.raw != null && m.normalized >= 60);
  if (!good.length) return null;
  return good.reduce((a, b) => (b.normalized > a.normalized ? b : a));
}
// Major faults are itemized per instant (like the Pacifist tab's table), but
// the same criterion can recur several times a match — group by criterion so
// a player who repeats one mistake gets one card with a count and times, not
// N visually-identical blocks.
function groupedMajorFaults(pac) {
  const byCriterion = new Map();
  for (const f of (pac ? pac.major_faults : []) || []) {
    const g = byCriterion.get(f.criterion) || { criterion: f.criterion, count: 0, times: [], detail: f.detail };
    g.count++;
    g.times.push(f.t);
    byCriterion.set(f.criterion, g);
  }
  return [...byCriterion.values()];
}
function playerImprovements(r, pac) {
  const items = [];
  groupedMajorFaults(pac).forEach(g => {
    const t = FAULT_TIPS[g.criterion];
    const times = g.times.map(x => fmt(x, 0) + "s").join(", ");
    items.push({
      sev: "major",
      label: `${g.criterion}${g.count > 1 ? ` × ${g.count}` : ""} — ${t ? t[0] : "Major fault"}`,
      tip: `${t ? t[1] : g.detail} (at ${times})`,
    });
  });
  if (r.main_leak && r.main_leak !== "none") {
    const t = METRIC_TIPS[r.main_leak];
    if (t) items.push({ sev: "leak", label: r.main_leak, chapter: t[0], tip: t[1] });
  }
  const second = secondWeakMetric(r);
  if (second) {
    const t = METRIC_TIPS[second.key];
    if (t) items.push({ sev: "secondary", label: second.key, chapter: t[0], tip: t[1] });
  }
  if (pac && pac.top_minor_fault) {
    const { criterion, count } = pac.top_minor_fault;
    const t = FAULT_TIPS[criterion];
    items.push({
      sev: "note",
      label: `${criterion} × ${count}${t ? ` — ${t[0]}` : ""} (most common Minor fault)`,
      tip: (t ? t[1] + " " : "") + "Minors don't cap the Pacifist score (only a Major does), but they're worth trimming — see the Pacifist tab's dimension breakdown for the full pattern.",
    });
  }
  return { items, strength: bestMetric(r) };
}
function renderImprove(d) {
  const pacByName = {};
  ((d.pacifist || {}).players || []).forEach(p => { pacByName[p.player] = p; });
  const scores = (d.scores || []).slice()
    .sort((a, b) => (a.target_team - b.target_team) || (a.target_pri - b.target_pri));
  const SEV_LABEL = { major: "Major fault", leak: "Leak", secondary: "Also work on", note: "Note" };
  const cards = scores.map(r => {
    const { items, strength } = playerImprovements(r, pacByName[r.target_player]);
    const rows = items.length
      ? items.map(it => `<div class="reco reco-${it.sev}">
          <div class="reco-h"><span class="reco-tag">${esc(SEV_LABEL[it.sev] || "")}</span>
            <b>${esc(it.label)}</b>${it.chapter ? `<span class="muted"> — ${esc(it.chapter)}</span>` : ""}</div>
          <div class="reco-tip">${esc(it.tip)}</div>
        </div>`).join("")
      : `<div class="reco muted">Nothing flagged — clean report this game.</div>`;
    const strengthRow = strength
      ? `<div class="reco reco-strength">
          <div class="reco-h"><span class="reco-tag">Strength</span><b>${esc(strength.key)}</b>
            <span class="muted"> — ${fmt(strength.normalized, 0)}/100</span></div>
          <div class="reco-tip">Best area this game — keep leaning on it.</div>
        </div>`
      : "";
    return `<div class="card"><p class="hint">${nameCell(r.target_team, r.target_player)}</p>${rows}${strengthRow}</div>`;
  }).join(`<div style="height:16px"></div>`);
  $("tab-improve").innerHTML =
    `<p class="muted" style="margin:0 0 16px;font-size:12.5px">Turns the existing scoring leak, ` +
    `Pacifist faults, and the next-worst metric into short, concrete "what to work on" notes per ` +
    `player — templated from the same rubric definitions the scores come from, not generated. A ` +
    `prioritized way into the Scoring and Pacifist tabs, not a replacement for them.</p>` +
    (cards || `<div class="card"><div class="reco muted">No players scored.</div></div>`);
}

// ---- Moments tab: the episodes behind the scores; a row jumps the 3D viewer to it ----
const mmss = t => Math.floor(t / 60) + ":" + String(Math.floor(t % 60)).padStart(2, "0");
async function seekViewer(t, tries = 50) {
  await selectTab("viewer");
  const w = $("viewerFrame")?.contentWindow;
  if (w?.seek) w.seek(t); else if (tries) setTimeout(() => seekViewer(t, tries - 1), 100);
}
const MISS = [[1, "low boost"], [2, "not facing the ball"], [4, "late"]];
const OUT = { goal: ["✓ goal", "good"], saved: ["saved", "warn"], off: ["off target", "muted"] };
// Shooter's view of the goal mouth (uu): 1785 wide, 643 high; dots are where each shot crossed the goal plane.
function shotMap(shots) {
  const dots = shots.filter(e => e.aim).map(e => `<circle cx="${Math.max(-1700, Math.min(1700, e.aim[0]))}" cy="${-e.aim[1]}" r="32" fill="var(--${OUT[e.outcome][1]})" opacity=".85"><title>${mmss(e.t)} · ${OUT[e.outcome][0]}</title></circle>`).join("");
  return `<div class="card"><p class="hint">Where each shot crossed the goal plane, as the shooter sees it — projected from the ball's post-touch velocity (bounces included), so treat it as approximate. ${shots.length} shots.</p>
    <svg viewBox="-1800 -800 3600 900" style="max-width:640px;width:100%;display:block;margin:0 auto"><rect x="-893" y="-643" width="1785" height="643" fill="none" stroke="var(--muted)" stroke-width="8"/>
    <line x1="-1800" y1="0" x2="1800" y2="0" stroke="var(--muted)" stroke-width="4"/>${dots}</svg></div>`;
}
// Per player: demos dealt/taken, seconds the victims were out, and goals within 8 s of a demo they dealt.
function physicality(d, who) {
  const demos = (d.episodes || []).filter(e => e.kind === "demo");
  if (!demos.length) return "";
  const rows = Object.values(who).map(r => {
    const dealt = demos.filter(e => e.pri === r.target_pri);
    return `<tr><td>${nameCell(r.target_team, r.target_player)}</td><td class="num">${dealt.length}</td>
      <td class="num">${demos.filter(e => e.victim === r.target_pri).length}</td>
      <td class="num">${fmt(dealt.reduce((s, e) => s + e.down, 0), 1)} s</td><td class="num">${dealt.filter(e => e.goal).length}</td></tr>`;
  }).join("");
  return cardTable("Physicality — demolitions from the replay's own events. “Opponents down” is how long the victims were out of the match; “goals after” counts your team scoring within 8 s of one of your demos.",
    `<th>Player</th><th class="num">Dealt</th><th class="num">Taken</th><th class="num">Opponents down</th><th class="num">Goals after</th>`, rows, "", 5);
}
const momentCols = (e, who) => e.kind === "demo"
  ? ["Demo of " + esc(who[e.victim]?.target_player ?? "?"), e.down > 0 ? `${e.goal ? "✓ goal within 8 s · " : ""}out ${fmt(e.down, 1)} s` : "not seen leaving the field"]
  : e.kind === "shot"
  ? ["Shot · " + fmt(e.speed * 0.036, 0) + " km/h", `<span class="${e.outcome === "goal" ? "pos" : "muted"}">${OUT[e.outcome][0]}</span>`]
  : e.kind === "loss"
  ? ["Possession lost", `${e.danger >= 0.5 ? "✗ deep in own half" : e.danger > 0 ? "own half" : "midfield or beyond"} · opponent ${fmt(e.opp_dist / 100, 0)} m away`]
  : e.kind === "challenge"
  ? ["50/50 vs " + esc(who[e.opp]?.target_player ?? "?"),
     e.miss ? "✗ " + MISS.filter(([b]) => e.miss & b).map(m => m[1]).join(", ") : "✓ clean arrival"]
  : ["Recovery", e.done ? "✓ recovered" : "✗ not within the cap"];
function renderMoments(d) {
  const who = Object.fromEntries((d.scores || []).map(r => [r.target_pri, r]));
  const draw = pri => {
    const evs = (d.episodes || []).filter(e => who[e.pri] && (pri === "" || String(e.pri) === pri));
    const rows = evs.map(e => { const [what, res] = momentCols(e, who); return `<tr class="mrow" data-t="${e.t ?? e.t0}"><td class="num">${mmss(e.t ?? e.t0)}</td>
        <td>${nameCell(who[e.pri].target_team, who[e.pri].target_player)}</td><td>${what}</td>
        <td class="num">${e.dur == null ? "" : fmt(e.dur, 2) + " s"}</td><td>${res}</td></tr>`; }).join("");
    $("momentsBody").innerHTML = cardTable("Click a moment to jump to it in the 3D viewer.",
      `<th class="num">Time</th><th>Player</th><th>Moment</th><th class="num">Duration</th><th>Result</th>`,
      rows, "No moments.", 5);
    $("momentsBody").insertAdjacentHTML("afterbegin", physicality(d, who) + shotMap(evs.filter(e => e.kind === "shot")));
    document.querySelectorAll("#momentsBody .mrow").forEach(tr => tr.onclick = () => seekViewer(+tr.dataset.t));
  };
  $("tab-moments").innerHTML = `<div class="hist-bar"><select id="momentsWho"><option value="">All players</option>` +
    Object.values(who).map(r => `<option value="${r.target_pri}">${esc(r.target_player)}</option>`).join("") +
    `</select></div><div id="momentsBody"></div>`;
  $("momentsWho").onchange = e => draw(e.target.value);
  draw("");
}

function renderStats(d) {
  const ps = d.bc_stats || [];
  const nm = p => `<td>${nameCell(p.team, p.player)}</td><td><span class="badge t${p.team}">${teamName(p.team)}</span></td>`;
  const n = (v, dd = 0) => `<td class="num">${fmt(v, dd)}</td>`;   // 0-dp default
  const p1 = v => n(v, 1);                                         // 1-dp (percent)
  const gap = `<div style="height:16px"></div>`;

  // Core scoreboard (header truth + recomputed shooting %).
  const coreHead = `<th>Player</th><th>Team</th><th class="num">Goals</th><th class="num">Assists</th>
    <th class="num">Saves</th><th class="num">Shots</th><th class="num">Shooting %</th><th class="num">Score</th>`;
  const coreBody = (d.core || []).map(p => `<tr>${nm(p)}
    ${n(p.goals)}${n(p.assists)}${n(p.saves)}${n(p.shots)}${p1(p.shooting_pct)}${n(p.score)}</tr>`).join("");

  // Turnovers — possession-loss rate from the scoring touch sequence
  // (1 − possession_retention) plus outcome-weighted giveaways from the value
  // model's per-touch ΔV (a touch that moved your own team away from scoring).
  const impByPri = {}; (d.impact.players || []).forEach(p => impByPri[p.pri] = p);
  const posRet = r => { const mb = (r.metrics || []).find(x => x.key === "possession_retention"); return mb ? mb.raw : null; };
  const toHead = `<th>Player</th><th>Team</th><th class="num">Turnover %</th><th class="num">Touches</th>
    <th class="num">Giveaways</th><th class="num">Giveaway %</th><th class="num">P(goal) bled</th>`;
  const toBody = (d.scores || []).slice()
    .sort((a, b) => (a.target_team - b.target_team) || (a.target_pri - b.target_pri))
    .map(r => {
      const pr = posRet(r);
      const to = (pr == null) ? "—" : fmt((1 - pr) * 100, 1);
      const imp = impByPri[r.target_pri];
      const tn = imp ? imp.touches : 0;
      const gv = imp ? imp.giveaways : 0;
      const gp = (imp && imp.touches > 0) ? fmt(gv / imp.touches * 100, 1) : "—";
      const ld = imp ? imp.lost_dv : 0;
      return `<tr><td>${nameCell(r.target_team, r.target_player)}</td>
        <td><span class="badge t${r.target_team}">${teamName(r.target_team)}</span></td>
        <td class="num">${to}</td><td class="num">${fmt(tn, 0)}</td><td class="num">${fmt(gv, 0)}</td>
        <td class="num">${gp}</td><td class="num neg">${ld ? "−" + fmt(ld, 3) : fmt(0, 3)}</td></tr>`;
    }).join("");

  // Boost economy.
  const boostHead = `<th>Player</th><th>Team</th><th class="num">BPM</th><th class="num">BCPM</th>
    <th class="num">Avg</th><th class="num">Collected</th><th class="num">Stolen</th>
    <th class="num">Big</th><th class="num">Small</th><th class="num">Overfill</th>
    <th class="num">0%</th><th class="num">100%</th>`;
  const boostBody = ps.map(p => { const b = p.boost; return `<tr>${nm(p)}
    ${n(b.bpm)}${n(b.bcpm)}${n(b.avg_amount)}${n(b.amount_collected)}${n(b.amount_stolen)}
    ${n(b.count_collected_big)}${n(b.count_collected_small)}${n(b.amount_overfill)}
    ${p1(b.percent_zero)}${p1(b.percent_full)}</tr>`; }).join("");

  // Movement (+ demos).
  const moveHead = `<th>Player</th><th>Team</th><th class="num">Avg spd</th><th class="num">Dist</th>
    <th class="num">Slow%</th><th class="num">Boost%</th><th class="num">Super%</th>
    <th class="num">Ground%</th><th class="num">Low air%</th><th class="num">High air%</th>
    <th class="num">PS</th><th class="num">Rev%</th><th class="num">Demo +/−</th>`;
  const moveBody = ps.map(p => { const m = p.movement, dm = p.demo; return `<tr>${nm(p)}
    ${n(m.avg_speed)}${n(m.total_distance)}${p1(m.percent_slow)}${p1(m.percent_boost_speed)}
    ${p1(m.percent_supersonic)}${p1(m.percent_ground)}${p1(m.percent_low_air)}${p1(m.percent_high_air)}
    ${n(m.count_powerslide)}${p1(m.percent_reverse)}<td class="num">${dm.inflicted}/${dm.taken}</td></tr>`; }).join("");

  // Positioning (team attack frame).
  const posHead = `<th>Player</th><th>Team</th><th class="num">Dist ball</th><th class="num">Dist mates</th>
    <th class="num">Def⅓</th><th class="num">Neut⅓</th><th class="num">Off⅓</th>
    <th class="num">Behind%</th><th class="num">Most back%</th><th class="num">Facing%</th>
    <th class="num">GA last-def</th>`;
  const posBody = ps.map(p => { const q = p.positioning; return `<tr>${nm(p)}
    ${n(q.avg_dist_to_ball)}${n(q.avg_dist_to_mates)}${p1(q.percent_defensive_third)}
    ${p1(q.percent_neutral_third)}${p1(q.percent_offensive_third)}${p1(q.percent_behind_ball)}
    ${p1(q.percent_most_back)}${p1(q.percent_facing_ball)}<td class="num">${q.goals_against_while_last_defender}</td></tr>`; }).join("");

  $("tab-stats").innerHTML =
    cardTable("Core scoreboard — goals/assists/saves/shots/score (header truth) plus shooting % (goals ÷ shots). These are game-credited outcomes, not reconstructed.",
      coreHead, coreBody, "No data.", 8) + gap +
    cardTable("Turnovers — Turnover % is how often your touch is followed by an opponent touch (1 − possession retention). Giveaways are touches that lowered your own team's chance of scoring next (negative ΔV); P(goal) bled sums how much scoring probability those gave away. Turnover % is descriptive (near-noise for rank); the ΔV columns weight a turnover by how much it actually cost.",
      toHead, toBody, "No data.", 7) + gap +
    cardTable("Boost economy — collected/used per minute, average gauge, pads collected (big/small), boost stolen in the opponent half & overfill, and time at empty / full. Ballchasing-parity aggregates (analyze::bcstats).",
      boostHead, boostBody, "No data.", 12) + gap +
    cardTable("Movement — average speed & total distance, speed-bucket shares (slow / boost / supersonic), air vs ground, powerslides, reverse-driving share, and demos inflicted / taken.",
      moveHead, moveBody, "No data.", 13) + gap +
    cardTable("Positioning (in each team's attack frame) — distance to ball & teammates, field-third occupancy, time behind the ball, last-defender & facing-the-ball shares, and goals conceded while last defender.",
      posHead, posBody, "No data.", 11);
}

function renderSkills(d) {
  const cols = [];
  (d.skill_profiles || []).forEach(p => Object.keys(p.skills).forEach(k => { if (!cols.includes(k)) cols.push(k); }));
  const head = `<th>Player</th><th>Team</th><th class="num">Total/min</th>` +
    cols.map(c => `<th class="num">${esc(c)}</th>`).join("");
  const rows = (d.skill_profiles || []).map(p => {
    const cells = cols.map(c => {
      const st = p.skills[c];
      return `<td class="num">${st ? `<span class="kc on">${st.count}</span>` : `<span class="kc off">·</span>`}</td>`;
    }).join("");
    return `<tr>
      <td>${nameCell(p.team, p.player)}</td>
      <td><span class="badge t${p.team}">${teamName(p.team)}</span></td>
      <td class="num">${fmt(p.total_per_min, 1)}</td>${cells}
    </tr>`;
  }).join("");
  $("tab-skills").innerHTML = cardTable(
    "Mechanical skills detected from kinematics (counts per player). Heuristic — thresholds are versioned.",
    head, rows, "No skills detected.", cols.length + 3);
}

function renderImpact(d) {
  const players = d.impact.players || [];
  const max = players.reduce((m, p) => Math.max(m, Math.abs(p.sum_dv)), 0);
  const rows = players.map(p => {
    const pct = max > 0 ? Math.min(100, Math.abs(p.sum_dv) / max * 100) / 2 : 0;
    const fill = p.sum_dv >= 0
      ? `left:50%;width:${pct}%;background:var(--good);`
      : `right:50%;width:${pct}%;background:var(--bad);`;
    return `<tr>
      <td>${nameCell(p.team, p.player || "—")}</td>
      <td><span class="badge t${p.team}">${teamName(p.team)}</span></td>
      <td class="num">${p.touches}</td>
      <td class="num"><span class="dv"><b class="${p.sum_dv >= 0 ? "pos" : "neg"}">${signed(p.sum_dv, 3)}</b>
        <span class="dvbar"><i style="${fill}"></i></span></span></td>
      <td class="num ${p.mean_dv >= 0 ? "pos" : "neg"}">${signed(p.mean_dv, 4)}</td>
    </tr>`;
  }).join("");
  const head = `<th>Player</th><th>Team</th><th class="num">Touches</th>
    <th class="num">Total ΔV</th><th class="num">Mean ΔV</th>`;
  $("tab-impact").innerHTML = cardTable(
    `Value model (ΔV): each player's summed per-touch swing in P(their team scores next). Trained in-process on this match; an independent cross-check on the scoring rubric. Base rate ${fmt(d.impact.base_rate, 3)}, log-loss ${fmt(d.impact.log_loss, 3)}.`,
    head, rows, "No value data.", 5);
}

function renderPacifist(d) {
  const pac = d.pacifist || { players: [] };
  const players = pac.players || [];
  const gap = `<div style="height:16px"></div>`;

  // Headline: score + FM-1 verdict per player.
  const sumHead = `<th>Player</th><th>Team</th><th class="num">Pacifist score</th>
    <th class="num">Confidence</th><th>Verdict</th><th class="num">Minors</th><th class="num">Majors</th>`;
  const sumBody = players.map(p => {
    const val = p.value == null
      ? `<span class="muted">—</span>`
      : `<div class="big">${fmt(p.value)}</div><div class="meter"><i style="width:${clampPct(p.value)}%"></i></div>`;
    const verdict = p.verdict === "PASS"
      ? `<b class="pos">PASS</b>`
      : `<b class="neg">FAIL</b>`;
    return `<tr>
      <td>${nameCell(p.team, p.player)}</td>
      <td><span class="badge t${p.team}">${teamName(p.team)}</span></td>
      <td class="num">${val}</td>
      <td class="num">${fmt(p.confidence * 100, 0)}%</td>
      <td>${verdict}</td>
      <td class="num">${p.minors}</td>
      <td class="num ${p.majors > 0 ? "neg" : ""}">${p.majors}</td>
    </tr>`;
  }).join("");

  // Dimension matrix: one column per rubric dimension, zero-confidence cells dimmed.
  const dims = [];
  players.forEach(p => (p.dimensions || []).forEach(x => { if (!dims.includes(x.label)) dims.push(x.label); }));
  const dimHead = `<th>Player</th><th>Team</th>` + dims.map(c => `<th class="num">${esc(c)}</th>`).join("");
  const dimBody = players.map(p => {
    const byLabel = {}; (p.dimensions || []).forEach(x => byLabel[x.label] = x);
    const cells = dims.map(c => {
      const x = byLabel[c];
      if (!x || x.confidence <= 0) return `<td class="num"><span class="kc off">·</span></td>`;
      return `<td class="num">${fmt(x.value, 0)}</td>`;
    }).join("");
    return `<tr><td>${nameCell(p.team, p.player)}</td>
      <td><span class="badge t${p.team}">${teamName(p.team)}</span></td>${cells}</tr>`;
  }).join("");

  // Major faults — each one is an instant verdict failure, so each gets a line.
  const mfHead = `<th>Player</th><th>Team</th><th class="num">Time</th><th>Criterion</th><th>What happened</th>`;
  const mfBody = players.flatMap(p => (p.major_faults || []).map(f => `<tr>
    <td>${nameCell(p.team, p.player)}</td>
    <td><span class="badge t${p.team}">${teamName(p.team)}</span></td>
    <td class="num">${fmt(f.t, 1)}s</td>
    <td><b class="neg">${esc(f.criterion)}</b></td>
    <td class="muted">${esc(f.detail)}</td>
  </tr>`)).join("");

  $("tab-pacifist").innerHTML =
    cardTable(
      `Pacifist system adherence (${esc(pac.config_version || "")}) — how closely each player follows the Pacifist positional system: ` +
      `a confidence-weighted blend of eight discipline dimensions, judged per opportunity (engagements, covers, challenges, shots). ` +
      `The verdict is the guide's FM-1 driving test: up to 15 Minor faults pass; one Major fault fails and caps the score. ` +
      `This measures adherence to a specific system, not rank.`,
      sumHead, sumBody, "No Pacifist data.", 7) + gap +
    cardTable(
      "Per-dimension values (0–100, higher = more disciplined). A dot means the dimension never applied to this player — no opportunities, so it carries no weight.",
      dimHead, dimBody, "No dimension data.", dims.length + 2) + gap +
    cardTable(
      "Major faults — the FM-2 shape: committed as last man, on an empty tank, against a ball the team does not own. Each is an instant verdict failure.",
      mfHead, mfBody, "No Major faults — nobody committed the unrecoverable dive.", 5);
}

// ---- tabs (lazy iframes for the heavy HTML views) ----
// A panel's HTML: inline in a static bundle, otherwise fetched once from the server.
async function panelHtml(name) {
  if (DATA[name + "_html"]) return DATA[name + "_html"];
  const r = await authFetch(`/api/analysis/${encodeURIComponent(DATA.analysis_id)}/${name}`);
  if (!r.ok) throw new Error(await r.text());
  return r.text();
}
const PANELS = {
  viewer: h => `<div class="vtoolbar"><button class="btn" id="fsBtn" type="button">⛶ Full screen</button>
      <span class="muted">Press Esc to exit full screen.</span></div>
      <iframe class="viewer" id="viewerFrame" allow="fullscreen" allowfullscreen srcdoc="${esc(h)}"></iframe>`,
  scoring: h => `<iframe class="report" srcdoc="${esc(h)}"></iframe>`,
  ballchasing: h => `<iframe class="report" srcdoc="${esc(h)}"></iframe>`,
};
const panelLoaded = {};   // reset on each analysis
async function selectTab(name) {
  document.querySelectorAll("nav.tabs button").forEach(b =>
    b.classList.toggle("active", b.dataset.tab === name));
  document.querySelectorAll(".tab").forEach(t =>
    t.classList.toggle("active", t.id === "tab-" + name));
  if (!PANELS[name] || panelLoaded[name]) return;
  panelLoaded[name] = true;
  const tab = $("tab-" + name);
  tab.innerHTML = `<p class="muted"><span class="spinner"></span>Loading…</p>`;
  try {
    tab.innerHTML = PANELS[name](await panelHtml(name));
    if (name === "viewer") $("fsBtn").onclick = enterViewerFullscreen;
  } catch (e) {
    panelLoaded[name] = false;
    tab.innerHTML = `<p class="muted">Could not load this panel: ${esc(e.message || e)}</p>`;
  }
}

// Take the 3D viewer iframe full screen (the canvas resizes to fill it).
function enterViewerFullscreen() {
  const frame = $("viewerFrame");
  if (!frame) return;
  const req = frame.requestFullscreen || frame.webkitRequestFullscreen || frame.msRequestFullscreen;
  if (!req) { setStatus("Full screen isn't supported by this browser.", true); return; }
  Promise.resolve(req.call(frame)).catch(err =>
    setStatus("Full screen unavailable: " + esc(err.message || err), true));
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

// ---- history: saved matches + cross-match habits (needs the server's --data-dir) ----
function getToken() { try { return localStorage.getItem("rleval.token") || ""; } catch (e) { return ""; } }
function setToken(t) { try { localStorage.setItem("rleval.token", t); } catch (e) { /* private mode: token lasts this page only */ } }
function authFetch(url, opts = {}, quiet = false) {
  const t = getToken();
  const headers = Object.assign({}, opts.headers, t ? { Authorization: "Bearer " + t } : {});
  return fetch(url, Object.assign({}, opts, { headers })).then(r => {
    // Surface the token field. Only reveal it — reloading history here would
    // re-enter authFetch and loop on every 401.
    if (r.status === 401 && !quiet) $("history").classList.add("open");
    return r;
  });
}
$("histLink").onclick = (e) => {
  e.preventDefault();
  const open = $("history").classList.toggle("open");
  if (open) { loadTeams(); loadHistory(); }
};
$("tokenInput").value = getToken();
$("tokenSave").onclick = () => { setToken($("tokenInput").value.trim()); loadTeams(); loadHistory(); };

// ---- sign-in: only shown when the server was started with --oidc-* ----
async function loadWho() {
  try {
    const info = await (await fetch("/api/auth")).json();
    if (!info.sign_in) return;
    const me = await fetch("/api/me");
    const show = (id, on) => { $(id).style.display = on ? "" : "none"; };
    if (me.ok) {
      $("who").textContent = (await me.json()).account;
      show("who", true); show("signOut", true); show("signIn", false);
    } else {
      show("signIn", true); show("signOut", false); show("who", false);
    }
  } catch (e) { /* sign-in is optional; the app works without it */ }
}
$("signOut").onclick = async (e) => {
  e.preventDefault();
  await fetch("/auth/logout", { method: "POST" });
  location.reload();
};

// ---- teams: shared match pool + roster view (needs --teams on the server) ----
// The optional query params of an analyze call: share team and declared rank.
function teamParam(prefix) {
  const q = [["team", $("uploadTeam").value], ["rank", $("uploadRank").value]]
    .filter(([, v]) => v).map(([k, v]) => k + "=" + encodeURIComponent(v));
  return q.length ? prefix + q.join("&") : "";
}
let TEAMS = [];
async function loadTeams(quiet = false) {
  const box = $("teamBody");
  try {
    const r = await authFetch("/api/teams", {}, quiet);
    TEAMS = r.ok ? await r.json() : [];
  } catch (e) { TEAMS = []; }
  const sel = $("uploadTeam"), keep = sel.value;
  sel.innerHTML = `<option value="">just me</option>` +
    TEAMS.map(t => `<option value="${esc(t.team)}">${esc(t.team)}</option>`).join("");
  sel.value = TEAMS.some(t => t.team === keep) ? keep : "";
  $("teamPick").style.display = TEAMS.length ? "" : "none";
  if (!TEAMS.length) { box.innerHTML = ""; return; }
  box.innerHTML =
    `<div class="hist-bar"><span class="muted">Team</span><select id="teamSel">` +
    TEAMS.map(t => `<option value="${esc(t.team)}">${esc(t.team)} (${esc(t.role)})</option>`).join("") +
    `</select></div><div id="teamReport"></div><div style="height:22px"></div>`;
  $("teamSel").onchange = () => loadTeamReport($("teamSel").value);
  loadTeamReport($("teamSel").value);
}
async function loadTeamReport(team) {
  const box = $("teamReport");
  try {
    const r = await authFetch("/api/teams/" + encodeURIComponent(team));
    if (!r.ok) throw new Error(await r.text());
    box.innerHTML = renderTeam(await r.json());
  } catch (e) {
    box.innerHTML = `<div class="card"><div class="reco reco-major">${esc(e.message || e)}</div></div>`;
  }
}
function renderTeam(t) {
  const rows = t.members.map(m => `<tr><td><b>${esc(m.account)}</b></td><td>${esc(m.role)}</td>
    <td>${esc(m.in_game)}</td><td class="num">${m.matches}</td><td class="num">${m.wins}–${m.losses}</td>
    <td>${m.focus ? esc(m.focus.dimension) : `<span class="muted">—</span>`}</td></tr>`).join("");
  const scope = t.viewer_role === "coach" ? "Every member of the roster." : "Just you — coaches see the full roster.";
  const roster = `<div class="card"><p class="hint" style="padding:12px 18px 0">${scope}</p><div class="tablewrap"><table>
    <tr><th>Account</th><th>Role</th><th>In-game name</th><th class="num">Matches</th><th class="num">W–L</th>
    <th>Work on</th></tr>${rows}</table></div></div>`;
  const empty = `<div class="card"><div class="reco muted">No shared matches yet — pick this team under
    “Share with team” when you analyze a replay.</div></div>`;
  return `<h3 style="margin:0 0 10px">${esc(t.team)} — ${t.sessions} shared match${t.sessions === 1 ? "" : "es"}</h3>` +
    (t.rollup ? renderHabits(t.rollup).replace(/^<h3[^>]*>.*?<\/h3>/, "") : empty) +
    `<div style="height:16px"></div>` + roster;
}

async function loadHistory() {
  const box = $("historyBody");
  box.innerHTML = `<div class="card"><div class="reco muted">Loading…</div></div>`;
  try {
    const r = await authFetch("/api/history");
    if (!r.ok) throw new Error(await r.text());
    const h = await r.json();
    if (!h.sessions.length) {
      box.innerHTML = `<div class="card"><div class="reco muted">No saved matches yet — analyze a replay and it is saved here.</div></div>`;
      return;
    }
    const opts = h.players.map(p => `<option value="${esc(p.key)}">${esc(p.name)} (${p.matches})</option>`).join("");
    box.innerHTML =
      `<div class="hist-bar"><span class="muted">${h.sessions.length} saved match${h.sessions.length === 1 ? "" : "es"} · player</span>` +
      `<select id="habitPlayer">${opts}</select></div><div id="habitBody"></div>`;
    $("habitPlayer").onchange = () => loadHabits($("habitPlayer").value);
    loadHabits($("habitPlayer").value);
  } catch (e) {
    box.innerHTML = `<div class="card"><div class="reco reco-major">${esc(e.message || e)}</div></div>`;
  }
}

async function loadHabits(player) {
  const box = $("habitBody");
  try {
    const r = await authFetch("/api/history/habits?player=" + encodeURIComponent(player));
    if (!r.ok) throw new Error(await r.text());
    box.innerHTML = renderHabits(await r.json());
  } catch (e) {
    box.innerHTML = `<div class="card"><div class="reco reco-major">${esc(e.message || e)}</div></div>`;
  }
}

function renderHabits(h) {
  const gap = v => v == null ? "—" : `<span class="${v >= 0 ? "gappos" : "gapneg"}">${signed(v, 0)}</span>`;
  const focus = h.focus
    ? `<div class="card focus"><div class="reco"><div class="reco-h"><span class="reco-tag">${
        h.focus.kind === "loss_habit" ? "Work on this next — shows up in your losses" : "Work on this next"}</span>
        <b>${esc(h.focus.dimension)}</b></div><div class="reco-tip">${esc(h.focus.reason)}</div></div></div>`
    : `<div class="card"><div class="reco muted">No scoreable Pacifist data for this player yet.</div></div>`;
  const fault = h.recurring_fault
    ? `<div class="reco reco-leak"><div class="reco-h"><span class="reco-tag">Recurring fault</span>
        <b>${esc(h.recurring_fault.criterion)}</b></div><div class="reco-tip">Your most frequent Minor fault in
        ${h.recurring_fault.matches} of ${h.matches} matches.</div></div>` : "";
  const rows = h.dimensions.map(d => `<tr><td>${esc(d.label)}</td>
    <td class="num">${fmt(d.overall, 0)}</td><td class="num">${fmt(d.in_wins, 0)}</td>
    <td class="num">${fmt(d.in_losses, 0)}</td><td class="num">${gap(d.gap)}</td>
    <td class="num">${d.opportunities}</td></tr>`).join("");
  const bars = h.trend.map(t => `<i class="${t.won === true ? "w" : t.won === false ? "l" : ""}"
    style="height:${Math.max(4, clampPct(t.value)) }%" title="${esc(t.label)}: ${fmt(t.value, 0)}"></i>`).join("");
  const note = (h.wins < 2 || h.losses < 2)
    ? `<p class="muted" style="font-size:12.5px">Win-vs-loss habits need at least 2 wins and 2 losses; ` +
      `you have ${h.wins} and ${h.losses}. Until then the focus is your weakest dimension.</p>` : "";
  return `<h3 style="margin:0 0 10px">${esc(h.player)} — ${h.matches} match${h.matches === 1 ? "" : "es"} ` +
    `(${h.wins}W ${h.losses}L)</h3>` + focus + note +
    `<div style="height:16px"></div><div class="card"><div class="tablewrap"><table>
      <tr><th>Dimension</th><th class="num">Overall</th><th class="num">In wins</th><th class="num">In losses</th>
      <th class="num">Win − loss</th><th class="num">Opps</th></tr>${rows}</table></div>${fault}</div>` +
    `<div style="height:16px"></div><div class="card"><p class="hint" style="padding:12px 18px 0">Pacifist score per match, ` +
    `oldest → newest (green = win, red = loss)</p><div class="trend">${bars}</div></div>`;
}

loadSamples();
loadWho();
loadTeams(true);   // reveals "Share with team" if this account is on any team
</script>
</body>
</html>
"##;

/// The model/config admin page, served at `/admin`. Read-only: fetches
/// [`crate::admin::config_report`] from `/api/config` and renders the scoring
/// rubric, value model, skills catalog, and corpus status.
pub const ADMIN_HTML: &str = r##"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>RLEval — Model Admin</title>
<style>
  :root {
    --bg:#0b0e14; --bg-soft:#11151d; --card:#141a23; --card-2:#1a212c;
    --line:#243040; --line-soft:#1b2330; --fg:#e8eef6; --muted:#93a1b5; --faint:#687586;
    --accent:#5b9dff; --accent-2:#8a6bff; --blue:#4f9dfd; --orange:#ff9f45;
    --good:#46d18b; --bad:#ff6b6b; --warn:#e3b341; --radius:14px;
    --shadow:0 1px 0 rgba(255,255,255,.03), 0 10px 30px -16px rgba(0,0,0,.7);
    --mono:ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  }
  * { box-sizing:border-box; }
  body { margin:0; color:var(--fg); font:14px/1.55 system-ui,-apple-system,"Segoe UI",Roboto,sans-serif;
    background:radial-gradient(1100px 560px at 12% -12%,rgba(138,107,255,.12),transparent 60%),
      radial-gradient(1000px 520px at 102% -4%,rgba(91,157,255,.12),transparent 56%),var(--bg);
    background-attachment:fixed; }
  a { color:var(--accent); text-decoration:none; }
  header { position:sticky; top:0; z-index:30; background:rgba(11,14,20,.72); backdrop-filter:blur(10px);
    border-bottom:1px solid var(--line-soft); }
  .hd { max-width:1180px; margin:0 auto; display:flex; align-items:center; gap:13px; padding:13px 22px; }
  .logo { width:31px; height:31px; border-radius:9px; display:grid; place-items:center; font-weight:800;
    font-size:12px; color:#fff; background:linear-gradient(135deg,var(--accent),var(--accent-2));
    box-shadow:0 6px 18px -6px var(--accent); }
  .brand { font-weight:700; font-size:16px; } .brand small { color:var(--muted); font-weight:500; font-size:12px; margin-left:7px; }
  .spacer { flex:1; }
  .wrap { max-width:1180px; margin:0 auto; padding:26px 22px 64px; }
  h2 { font-size:15px; margin:30px 0 12px; display:flex; align-items:center; gap:10px; }
  h2 .ver { font:12px var(--mono); color:var(--accent); background:var(--card); border:1px solid var(--line);
    padding:3px 9px; border-radius:999px; font-weight:500; }
  .card { background:var(--card); border:1px solid var(--line); border-radius:var(--radius);
    box-shadow:var(--shadow); overflow:hidden; margin-bottom:14px; }
  .hint { padding:12px 18px; margin:0; color:var(--muted); font-size:12.5px;
    border-bottom:1px solid var(--line-soft); background:var(--bg-soft); }
  .tablewrap { overflow:auto; }
  table { width:100%; border-collapse:collapse; font-size:13.5px; }
  thead th { position:sticky; top:0; background:var(--card-2); color:var(--faint); font-weight:600;
    font-size:11px; text-transform:uppercase; letter-spacing:.5px; text-align:left; padding:10px 14px;
    border-bottom:1px solid var(--line); white-space:nowrap; }
  tbody td { padding:9px 14px; border-bottom:1px solid var(--line-soft); }
  tbody tr:last-child td { border-bottom:0; }
  td.num { text-align:right; font-variant-numeric:tabular-nums; font-family:var(--mono); }
  code { font-family:var(--mono); font-size:12.5px; color:#cfe2ff; }
  .badge { font-size:11px; padding:2px 9px; border-radius:999px; font-weight:600; border:1px solid transparent; }
  .b-first { color:var(--blue); background:rgba(79,157,253,.12); border-color:rgba(79,157,253,.25); }
  .b-second { color:var(--accent-2); background:rgba(138,107,255,.12); border-color:rgba(138,107,255,.25); }
  .b-general { color:var(--muted); background:rgba(147,161,181,.1); border-color:rgba(147,161,181,.22); }
  .exp { color:var(--warn); background:rgba(227,179,65,.12); border:1px solid rgba(227,179,65,.3);
    font-size:10px; padding:1px 7px; border-radius:999px; }
  .pad { padding:14px 18px; }
  .chips { display:flex; flex-wrap:wrap; gap:6px; }
  .chip { background:var(--card-2); border:1px solid var(--line); border-radius:999px; padding:4px 11px;
    font:12px var(--mono); color:var(--fg); }
  .kv { display:grid; grid-template-columns:max-content 1fr; gap:6px 16px; font-size:13px; }
  .kv .k { color:var(--faint); }
  .status { display:inline-flex; align-items:center; gap:7px; font-size:12.5px; }
  .ok { color:var(--good); } .warnc { color:var(--warn); } .muted { color:var(--muted); }
  .bar { height:8px; border-radius:999px; background:var(--line); overflow:hidden; min-width:120px; }
  .bar > i { display:block; height:100%; background:linear-gradient(90deg,var(--accent),var(--accent-2)); }
  .note { margin:12px 0 0; padding:11px 16px; border-radius:12px; font-size:12.5px; color:var(--warn);
    background:linear-gradient(90deg,rgba(227,179,65,.12),transparent); border:1px solid rgba(227,179,65,.3); }
  .imp { display:flex; align-items:center; gap:10px; margin:3px 0; }
  .imp code { width:150px; }
  .imp .bar { flex:1; }
  .imp .pct { width:48px; text-align:right; font-variant-numeric:tabular-nums; font-family:var(--mono); color:var(--muted); }
  .runbtn { background:var(--card-2); color:var(--fg); border:1px solid var(--line); border-radius:9px;
    padding:8px 14px; font-size:13px; font-weight:600; cursor:pointer; transition:.15s; }
  .runbtn:hover:not(:disabled) { border-color:var(--accent); color:#fff; background:rgba(91,157,255,.1); }
  .runbtn:disabled { opacity:.45; cursor:not-allowed; }
  #runout { margin-top:12px; padding:12px; background:#0a0d12; border:1px solid var(--line); border-radius:10px;
    font:12px/1.5 var(--mono); color:#c7d3e0; max-height:340px; overflow:auto; white-space:pre-wrap; }
</style>
</head>
<body>
<header><div class="hd">
  <div class="logo">RL</div>
  <div class="brand">RLEval <small>model admin</small></div>
  <div class="spacer"></div>
  <a href="/">← Analyze</a>
</div></header>
<div class="wrap" id="root"><p class="muted">Loading model config…</p></div>
<script>
const $ = s => document.querySelector(s);
const esc = s => String(s).replace(/[&<>"]/g, c => ({ "&":"&amp;","<":"&lt;",">":"&gt;","\"":"&quot;" }[c]));
const num = (v, d=2) => (v==null||isNaN(v)) ? "—" : Number(v).toFixed(d);

function curveStr(cv) {
  if (!cv) return "—";
  if (cv.kind === "higher") return `higher · ${num(cv.zero)} → ${num(cv.full)}`;
  if (cv.kind === "lower")  return `lower · ${num(cv.zero)} → ${num(cv.full)}`;
  if (cv.kind === "band")   return `band · [${num(cv.lo)}, ${num(cv.hi)}] ±${num(cv.falloff)}`;
  return esc(JSON.stringify(cv));
}
// Relative-age formatting for a Unix-epoch (seconds) mtime; >14 days reads stale.
function ago(epoch) {
  if (!epoch) return null;
  const s = Date.now() / 1000 - epoch, d = s / 86400;
  const t = s < 90 ? "just now" : s < 5400 ? Math.round(s/60) + "m ago"
    : s < 172800 ? Math.round(s/3600) + "h ago" : Math.round(d) + "d ago";
  return { text: t, stale: d > 14, date: new Date(epoch*1000).toISOString().slice(0,10) };
}
function modStr(o) {
  const a = o && ago(o.modified_epoch);
  if (!a) return "";
  return ` · <span class="${a.stale?'warnc':'muted'}" title="${a.date}">updated ${a.text}${a.stale?' ⚠ stale':''}</span>`;
}

async function runAction(act) {
  const out = document.getElementById("runout");
  out.style.display = "block";
  out.textContent = `Running ${act}… first run compiles in release mode — this can take a few minutes.`;
  document.querySelectorAll(".runbtn").forEach(b => b.disabled = true);
  try {
    const r = await fetch("/api/admin/run?action=" + encodeURIComponent(act), { method: "POST" });
    if (!r.ok) { out.textContent = "Error: " + esc(await r.text()); }
    else { const j = await r.json(); out.textContent = (j.ok ? "✓ " : "✗ ") + j.command + "\n\n" + j.output; }
  } catch (e) { out.textContent = "Error: " + esc(e.message || e); }
  document.querySelectorAll(".runbtn").forEach(b => b.disabled = false);
}
function card(hint, inner) {
  return `<div class="card"><p class="hint">${hint}</p>${inner}</div>`;
}

function renderScoring(s) {
  const c = s.config;
  const tw = c.top_weights || [];
  const onDisk = s.fitted
    ? `<span class="status ok">● fitted on disk: <code>${esc(s.fitted.version)}</code> <span class="muted">(${esc(s.fitted.path)})</span>${modStr(s.fitted)}</span>`
    : `<span class="status warnc">● no fitted_config.json — CLI tools would fall back to defaults</span>`;
  const gap = (s.fitted && s.fitted.version.indexOf(s.active_version) !== 0)
    ? `<p class="note">The app runs the in-process default (<code>${esc(s.active_version)}</code>); the fitted config on disk (<code>${esc(s.fitted.version)}</code>) is used by the <code>reconcile</code>/CLI tools, not the live web analysis.</p>` : "";
  // Rank-relative norms: the artifact the *live* app reads to grade vs rank.
  const norms = s.rank_norms
    ? `<span class="status ok">● rank norms: <code>${esc(s.rank_norms.version)}</code> <span class="muted">(${esc(s.rank_norms.path)})</span>${modStr(s.rank_norms)}</span>`
    : `<span class="status warnc">● no rank_norms.json — the app scores absolute only (no “vs rank” view); run calibrate to build it</span>`;
  const rows = (c.metrics || []).map(m => `<tr>
    <td><code>${esc(m.metric)}</code></td>
    <td><span class="badge b-${m.role}">${esc(m.role)}</span></td>
    <td>${esc(curveStr(m.curve))}</td>
    <td class="num">${num(m.weight, 2)}</td>
    <td>${m.experimental ? '<span class="exp">candidate</span>' : ""}</td>
    <td class="muted">${esc(m.chapter || "")}</td>
  </tr>`).join("");
  const metricTable = `<div class="tablewrap"><table>
    <thead><tr><th>Metric</th><th>Sub-score</th><th>Curve</th><th class="num">Weight</th><th>Flag</th><th>Chapter</th></tr></thead>
    <tbody>${rows}</tbody></table></div>`;
  const tiers = (c.tiers || []).map(t => `<span class="chip">${esc(t.name)} ≥ ${num(t.min_composite,0)}</span>`).join("");
  const cents = (c.centroids || []).map(ct => `<span class="chip">${esc(ct.name)} [${(ct.vector||[]).map(v=>num(v,2)).join(", ")}]</span>`).join("");
  return `<h2>Decision-discipline rubric <span class="ver">${esc(s.active_version)}</span></h2>` +
    card(`Active in-app: <code>${esc(s.active_version)}</code> &nbsp;·&nbsp; ${onDisk}<br>${norms}${gap}`, metricTable) +
    card("Role weights (1st / 2nd / general) and licence bands.",
      `<div class="pad"><div class="kv">
        <div class="k">Role weights</div><div>1st <code>${num(tw[0],2)}</code> · 2nd <code>${num(tw[1],2)}</code> · general <code>${num(tw[2],2)}</code></div>
        <div class="k">Licence bands</div><div class="chips">${tiers}</div>
        <div class="k">Archetypes</div><div class="chips">${cents}</div>
      </div></div>`);
}

function renderValue(v) {
  const names = v.feature_names || [];
  const feats = names.map((f,i) => `<span class="chip">${i}. ${esc(f)}</span>`).join("");
  const shipped = v.shipped
    ? `<span class="status ok">● ${esc(v.shipped.kind)} · n_train <code>${v.shipped.n_train.toLocaleString()}</code> <span class="muted">(${esc(v.shipped.path)})</span>${modStr(v.shipped)}</span>`
    : `<span class="status warnc">● value_model.json not present</span>`;
  // Shipped-GBT feature importance, sorted desc.
  let impCard = "";
  if (v.shipped && v.shipped.importance) {
    const rows = v.shipped.importance.map((w,i) => ({ name: names[i] || ("f"+i), w }))
      .sort((a,b) => b.w - a.w);
    const max = rows.length ? rows[0].w : 1;
    const bars = rows.map(r => `<div class="imp"><code>${esc(r.name)}</code>
      <div class="bar"><i style="width:${max>0?Math.round(r.w/max*100):0}%"></i></div>
      <span class="pct">${(r.w*100).toFixed(1)}%</span></div>`).join("");
    impCard = card("Shipped GBT feature importance — share of tree splits using each feature (a coarse 'weight' importance; the model stores no per-split gain).",
      `<div class="pad">${bars}</div>`);
  }
  return `<h2>Value / impact model <span class="ver">${esc(v.config_version)}</span></h2>` +
    card(`Runtime: ${esc(v.runtime)}<br>Shipped corpus model: ${shipped}`,
      `<div class="pad"><div class="kv"><div class="k">Features (${names.length})</div>
        <div class="chips">${feats}</div></div></div>`) + impCard;
}

function renderSkills(s) {
  const cat = (s.catalog||[]).map(c => `<span class="chip">${esc(c)}</span>`).join("");
  const onDisk = s.fitted
    ? `<span class="status ok">● fitted: <code>${esc(s.fitted.version)}</code>${modStr(s.fitted)}</span>`
    : `<span class="status muted">● no fitted skill config</span>`;
  return `<h2>Mechanical skills <span class="ver">${esc(s.active_version)}</span></h2>` +
    card(`Catalog of detectors. ${onDisk}`,
      `<div class="pad"><div class="chips">${cat}</div></div>`);
}

function renderCorpus(c) {
  if (!c.present) return `<h2>Calibration corpus</h2>` +
    card(`<span class="status warnc">● manifest not found at <code>${esc(c.manifest_path)}</code></span>`, "");
  const max = (c.per_bucket||[]).reduce((m,[,n]) => Math.max(m,n), 0) || 1;
  const rows = (c.per_bucket||[]).map(([b,n]) => `<tr>
    <td>${esc(b)}</td><td class="num">${n}</td>
    <td><div class="bar"><i style="width:${Math.round(n/max*100)}%"></i></div></td></tr>`).join("");
  return `<h2>Calibration corpus <span class="ver">${c.total} replays</span></h2>` +
    card(`Ranked-2v2 replays per rank tier (<code>${esc(c.manifest_path)}</code>). Replay files are gitignored; grow with <code>expand_manifest.py</code>.`,
      `<div class="tablewrap"><table><thead><tr><th>Bucket</th><th class="num">Count</th><th>Distribution</th></tr></thead>
        <tbody>${rows}</tbody></table></div>`);
}

function renderMaintenance(c) {
  const on = c.run_enabled;
  const btn = (act, label) => `<button class="runbtn" data-act="${act}" ${on?"":"disabled"}>${label}</button>`;
  const note = on
    ? "Runs the corpus jobs on this machine and writes the artifacts above. First run compiles in release mode (can take minutes); calibration needs the corpus downloaded, and fetching needs BC_TOKEN in the server's environment."
    : "Disabled. Restart the server with <code>--enable-admin-run</code> to turn these on (localhost only — it executes processes).";
  return `<h2>Maintenance</h2>` + card(note,
    `<div class="pad">
       <div style="display:flex;gap:10px;flex-wrap:wrap">
         ${btn("retrain-value","↻ Retrain value model")}
         ${btn("calibrate-scoring","↻ Recalibrate scoring")}
       </div>
       <pre id="runout" style="display:none"></pre>
     </div>`);
}

(async () => {
  try {
    const c = await (await fetch("/api/config")).json();
    $("#root").innerHTML = renderScoring(c.scoring) + renderValue(c.value) +
      renderSkills(c.skills) + renderCorpus(c.corpus) + renderMaintenance(c);
    document.querySelectorAll(".runbtn").forEach(b => b.onclick = () => runAction(b.dataset.act));
  } catch (e) {
    $("#root").innerHTML = `<p class="warnc">Failed to load /api/config: ${esc(e.message||e)}</p>`;
  }
})();
</script>
</body>
</html>
"##;
