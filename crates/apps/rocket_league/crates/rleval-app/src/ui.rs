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
      <div class="status" id="status"></div>
    </div>
  </div>

  <div id="summary">
    <div class="stats" id="meta"></div>
    <div class="nonstd" id="nonstd" style="display:none">
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"
           stroke-linecap="round" stroke-linejoin="round"><path d="M12 9v4M12 17h.01"/>
        <path d="M10.3 3.9 1.8 18a2 2 0 0 0 1.7 3h17a2 2 0 0 0 1.7-3L13.7 3.9a2 2 0 0 0-3.4 0Z"/></svg>
      Non-standard map — positional metrics assume standard Soccar, so scores are flagged low-confidence.
    </div>
    <nav class="tabs" id="tabs">
      <button data-tab="overview" class="active">Overview</button>
      <button data-tab="stats">Stats</button>
      <button data-tab="viewer">3D Viewer</button>
      <button data-tab="scoring">Scoring</button>
      <button data-tab="skills">Skills</button>
      <button data-tab="impact">Impact</button>
    </nav>
    <div class="tab active" id="tab-overview"></div>
    <div class="tab" id="tab-stats"></div>
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

  renderOverview(d);
  renderStats(d);
  renderSkills(d);
  renderImpact(d);
  // The heavy iframes are filled lazily on first tab open.
  viewerLoaded = scoringLoaded = false;
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

function renderOverview(d) {
  const impactByPri = {}; (d.impact.players || []).forEach(p => impactByPri[p.pri] = p);
  const skillByPri = {}; (d.skill_profiles || []).forEach(p => skillByPri[p.pri] = p);
  const rows = (d.scores || []).slice().sort((a, b) => b.composite - a.composite).map(r => {
    const imp = impactByPri[r.target_pri];
    const sk = skillByPri[r.target_pri];
    const dv = imp ? imp.sum_dv : null;
    return `<tr>
      <td>${nameCell(r.target_team, r.target_player)}</td>
      <td><span class="badge t${r.target_team}">${teamName(r.target_team)}</span></td>
      <td class="num"><div class="big">${fmt(r.composite)}</div>
        <div class="meter"><i style="width:${clampPct(r.composite)}%"></i></div></td>
      <td>${esc(r.licence)}</td>
      <td>${esc(r.player_type)}</td>
      <td class="num">${sk ? fmt(sk.total_per_min, 1) : "—"}</td>
      <td class="num ${dv >= 0 ? "pos" : "neg"}">${dv == null ? "—" : signed(dv, 3)}</td>
      <td class="muted">${esc(r.main_leak)}</td>
    </tr>`;
  }).join("");
  const head = `<th>Player</th><th>Team</th><th class="num">Composite</th><th>Licence</th>
    <th>Type</th><th class="num">Skills/min</th><th class="num">Impact ΔV</th><th>Main leak</th>`;
  $("tab-overview").innerHTML = cardTable(
    "One row per player — decision-discipline composite (scoring), mechanical activity (skills/min), and value impact (ΔV). Open the tabs for the full 3D replay, the scoring report, and per-skill detail.",
    head, rows, "No players scored.", 8);
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

// ---- tabs (lazy iframes for the heavy HTML views) ----
let viewerLoaded = false, scoringLoaded = false;
function selectTab(name) {
  document.querySelectorAll("nav.tabs button").forEach(b =>
    b.classList.toggle("active", b.dataset.tab === name));
  document.querySelectorAll(".tab").forEach(t =>
    t.classList.toggle("active", t.id === "tab-" + name));
  if (name === "viewer" && !viewerLoaded) {
    $("tab-viewer").innerHTML = `
      <div class="vtoolbar">
        <button class="btn" id="fsBtn" type="button">⛶ Full screen</button>
        <span class="muted">Press Esc to exit full screen.</span>
      </div>
      <iframe class="viewer" id="viewerFrame" allow="fullscreen" allowfullscreen
        srcdoc="${esc(DATA.viewer_html)}"></iframe>`;
    $("fsBtn").onclick = enterViewerFullscreen;
    viewerLoaded = true;
  }
  if (name === "scoring" && !scoringLoaded) {
    $("tab-scoring").innerHTML = `<iframe class="report" srcdoc="${esc(DATA.scoring_html)}"></iframe>`;
    scoringLoaded = true;
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

loadSamples();
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
    card(`Active in-app: <code>${esc(s.active_version)}</code> &nbsp;·&nbsp; ${onDisk}${gap}`, metricTable) +
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
