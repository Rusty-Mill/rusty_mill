//! Self-contained HTML 3D viewer for a [`Scene`].
//!
//! Mirrors `replay_scoring::render::html`: produces one openable `.html` file
//! with the scene data embedded, so the artifact is portable. Rendering uses
//! three.js loaded from a CDN (an import map), so **viewing needs network access**
//! for that one dependency; the match data itself is fully inline.

use crate::scene::Scene;

/// Render a scene to a complete, self-contained HTML document.
pub fn html(scene: &Scene) -> String {
    let json = serde_json::to_string(scene).expect("scene serializes");
    // Escape `< > &` so a player name can never break out of the <script> or
    // the embedded literal; the result is still valid JSON / JS.
    let safe = json
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026");
    let title = scene
        .replay_id
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    TEMPLATE
        .replace("/*SCENE_DATA*/", &safe)
        .replace("{{TITLE}}", &title)
}

const TEMPLATE: &str = r##"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{{TITLE}} — replay viewer</title>
<style>
  * { box-sizing: border-box; }
  html, body { margin: 0; height: 100%; overflow: hidden; background: #0b0e14;
    color: #e6edf3; font: 13px/1.4 ui-monospace, SFMono-Regular, Menlo, monospace; }
  #c { position: fixed; inset: 0; }
  .panel { position: fixed; background: rgba(13,17,23,.82); border: 1px solid #1f2937;
    border-radius: 8px; padding: 8px 10px; backdrop-filter: blur(3px); }
  button { background:#21262d; color:#e6edf3; border:1px solid #30363d; border-radius:6px;
    cursor:pointer; font:inherit; }
  button:hover { background:#2b333d; }
  #top { top: 10px; left: 50%; transform: translateX(-50%); text-align: center;
    font-size: 16px; font-weight: 700; }
  #top small { display:block; font-weight: 400; font-size: 11px; color:#8b949e; }
  #top #poss { font-size: 11px; font-weight: 700; }
  #players { top: 10px; left: 10px; min-width: 168px; }
  #players .row { cursor:pointer; padding:1px 3px; border-radius:4px; }
  #players .row:hover { background:#1c2230; }
  #players .row.follow { background:#243049; outline:1px solid #3b82f6; }
  #players .dot { display:inline-block; width:9px; height:9px; border-radius:50%;
    margin-right:6px; vertical-align:middle; }
  #players b { float:right; color:#c9d1d9; }
  #ticker { top: 10px; right: 10px; width: 244px; max-height: 44vh; overflow:hidden; }
  #ticker .ev { padding:1px 0; white-space:nowrap; overflow:hidden; text-overflow:ellipsis;
    cursor:pointer; }
  #ticker .ev:hover { color:#fff; }
  #ticker .ev span { color:#6e7681; margin-right:6px; }
  #ticker .goal { color:#ffd166; font-weight:700; }
  #ticker .demo { color:#f97316; }
  #ticker .skill { color:#56d4dd; }
  #ticker .kickoff { color:#a0aec0; }
  #ticker .touch { color:#8b949e; }
  #left { position:fixed; left:10px; bottom:44px; display:flex; flex-direction:column; gap:5px; align-items:flex-start; }
  #left .panel { position:static; }
  #left .cams { display:flex; gap:4px; }
  #left .cams button { padding:3px 7px; }
  #left .cams button.on { background:#243049; border-color:#3b82f6; }
  #left label { color:#c9d1d9; margin-right:8px; user-select:none; }
  #help { left:10px; bottom:10px; color:#6e7681; font-size:11px; }
  #bar { left: 50%; bottom: 14px; transform: translateX(-50%); display:flex;
    align-items:center; gap:10px; width: min(900px, 92vw); }
  #bar button.icon { width:34px; height:30px; display:flex; align-items:center; justify-content:center; }
  #bar select { background:#21262d; color:#e6edf3; border:1px solid #30363d; border-radius:6px; height:30px; }
  #clock { min-width: 116px; text-align:center; color:#c9d1d9; }
  #tlwrap { position:relative; flex:1; height:30px; display:flex; align-items:center; }
  #timeline { width:100%; }
  #marks { position:absolute; left:0; right:0; top:3px; height:10px; pointer-events:none; }
  #marks .mk { position:absolute; top:0; width:2px; height:10px; transform:translateX(-1px);
    pointer-events:auto; cursor:pointer; }
  #marks .mk.goal { background:#ffd166; height:14px; top:-2px; width:3px; }
  #marks .mk.demo { background:#f97316; }
  #marks .mk.kickoff { background:#5aa0c0; }
  #bar label { color:#8b949e; user-select:none; }
</style>
</head>
<body>
<canvas id="c"></canvas>
<div id="top" class="panel"><span id="scoreboard"></span><small id="mapline"></small><span id="poss"></span></div>
<div id="players" class="panel"></div>
<div id="ticker" class="panel"></div>
<div id="left">
  <div class="panel cams">
    <button data-cam="free" class="on">overview</button>
    <button data-cam="goal">goal</button>
    <button data-cam="ball">ball-cam</button>
  </div>
  <div class="panel">
    <label><input type="checkbox" id="tgTrails" checked> trails</label>
    <label><input type="checkbox" id="tgLabels" checked> labels</label>
    <label><input type="checkbox" id="tgBoost" checked> boost</label>
  </div>
</div>
<div id="help" class="panel">space play · ◀▶ ±1s · n/p next·prev goal · k/j next·prev kickoff · click a player to follow · 0 free cam</div>
<div id="bar" class="panel">
  <button id="play" class="icon" title="play/pause (space)"></button>
  <span id="clock">0:00.0</span>
  <div id="tlwrap"><input id="timeline" type="range" min="0" value="0"><div id="marks"></div></div>
  <select id="speed" title="playback speed">
    <option value="0.25">0.25×</option><option value="0.5">0.5×</option>
    <option value="1" selected>1×</option><option value="2">2×</option><option value="4">4×</option>
  </select>
  <label><input type="checkbox" id="loop"> loop</label>
</div>

<script type="importmap">
{ "imports": {
  "three": "https://cdn.jsdelivr.net/npm/three@0.160.0/build/three.module.js",
  "three/addons/": "https://cdn.jsdelivr.net/npm/three@0.160.0/examples/jsm/"
}}
</script>
<script type="module">
import * as THREE from 'three';
import { OrbitControls } from 'three/addons/controls/OrbitControls.js';

const S = /*SCENE_DATA*/;
const F = S.field;
const TEAM = { 0: 0x3b82f6, 1: 0xf97316 };
const TEAM_CSS = { 0: '#3b82f6', 1: '#f97316' };
const ICON_PLAY = '<svg width="14" height="14" viewBox="0 0 14 14"><path d="M3 2l9 5-9 5z" fill="currentColor"/></svg>';
const ICON_PAUSE = '<svg width="14" height="14" viewBox="0 0 14 14"><rect x="3" y="2" width="3" height="10" fill="currentColor"/><rect x="8" y="2" width="3" height="10" fill="currentColor"/></svg>';

THREE.Object3D.DEFAULT_UP.set(0, 0, 1); // Rocket League is Z-up

const scene = new THREE.Scene();
scene.background = new THREE.Color(0x0b0e14);
scene.fog = new THREE.Fog(0x0b0e14, F.back_wall_y * 2.4, F.back_wall_y * 5);

const camera = new THREE.PerspectiveCamera(55, innerWidth / innerHeight, 10, 60000);
const renderer = new THREE.WebGLRenderer({ antialias: true, canvas: document.getElementById('c') });
renderer.setSize(innerWidth, innerHeight);
renderer.setPixelRatio(Math.min(devicePixelRatio, 2));

const controls = new OrbitControls(camera, renderer.domElement);
controls.enableDamping = true;
controls.maxDistance = F.back_wall_y * 4;

scene.add(new THREE.HemisphereLight(0xb8d0ff, 0x202830, 1.15));
const dir = new THREE.DirectionalLight(0xffffff, 1.25);
dir.position.set(2500, -3500, 7000);
scene.add(dir);

// --- camera presets ---
const OVERVIEW = { pos: new THREE.Vector3(0, -F.back_wall_y * 1.22, F.ceiling_z * 2.0), tgt: new THREE.Vector3(0, 0, 150) };
const GOALVIEW = { pos: new THREE.Vector3(0, -F.back_wall_y * 1.5, F.ceiling_z * 1.1), tgt: new THREE.Vector3(0, F.back_wall_y * 0.3, 200) };
let cam = { mode: 'free', pri: null };
function applyPose(p) { camera.position.copy(p.pos); controls.target.copy(p.tgt); controls.update(); }
applyPose(OVERVIEW);

function setCam(mode, pri) {
  cam = { mode, pri: pri ?? null };
  document.querySelectorAll('#left .cams button').forEach(b => b.classList.toggle('on', b.dataset.cam === mode));
  if (mode === 'free') applyPose(OVERVIEW);
  else if (mode === 'goal') applyPose(GOALVIEW);
  updatePlayers(lastState); // refresh follow highlight
}
document.querySelectorAll('#left .cams button').forEach(b => b.onclick = () => setCam(b.dataset.cam));

buildField();
const ball = makeBall();
const ballShadow = makeShadow(F.ball_radius * 1.1);
const cars = buildCars();

function buildField() {
  const fx = F.side_wall_x, fy = F.back_wall_y, fz = F.ceiling_z;
  const floor = new THREE.Mesh(
    new THREE.PlaneGeometry(2 * fx, 2 * fy),
    new THREE.MeshStandardMaterial({ color: 0x12351f, roughness: 1 }));
  scene.add(floor);
  const walls = new THREE.LineSegments(
    new THREE.EdgesGeometry(new THREE.BoxGeometry(2 * fx, 2 * fy, fz)),
    new THREE.LineBasicMaterial({ color: 0x2b4d6f }));
  walls.position.set(0, 0, fz / 2);
  scene.add(walls);
  const lineMat = new THREE.LineBasicMaterial({ color: 0x3a5a78 });
  scene.add(new THREE.Line(new THREE.BufferGeometry().setFromPoints(
    [new THREE.Vector3(-fx, 0, 2), new THREE.Vector3(fx, 0, 2)]), lineMat));
  const circ = new THREE.Mesh(new THREE.RingGeometry(900, 920, 48),
    new THREE.MeshBasicMaterial({ color: 0x3a5a78, side: THREE.DoubleSide }));
  circ.position.z = 2; scene.add(circ);
  for (const sign of [1, -1]) {
    const g = new THREE.LineSegments(
      new THREE.EdgesGeometry(new THREE.BoxGeometry(2 * F.goal_half_width, 200, F.goal_height)),
      new THREE.LineBasicMaterial({ color: 0xffd166 }));
    g.position.set(0, sign * fy, F.goal_height / 2); scene.add(g);
  }
}

function makeBall() {
  const m = new THREE.Mesh(new THREE.SphereGeometry(F.ball_radius, 28, 18),
    new THREE.MeshStandardMaterial({ color: 0xeaeaea, emissive: 0x303030, roughness: .4 }));
  scene.add(m); return m;
}

// A flat blob shadow on the floor; opacity/scale track the object's height.
function makeShadow(r) {
  const m = new THREE.Mesh(new THREE.CircleGeometry(r, 24),
    new THREE.MeshBasicMaterial({ color: 0x000000, transparent: true, opacity: .35, depthWrite: false }));
  m.position.z = 1.5; scene.add(m); return m;
}
function placeShadow(sh, x, y, z) {
  sh.position.set(x, y, 1.5);
  const h = Math.max(0, (z - 17) / F.ceiling_z);
  sh.material.opacity = Math.max(0.05, 0.38 * (1 - h));
  const s = 1 + h * 1.2; sh.scale.set(s, s, 1);
}

function makeLabel(text, hex) {
  const cv = document.createElement('canvas'); cv.width = 256; cv.height = 64;
  const x = cv.getContext('2d');
  x.fillStyle = 'rgba(0,0,0,.55)'; x.fillRect(0, 0, 256, 64);
  x.fillStyle = '#' + hex.toString(16).padStart(6, '0'); x.fillRect(0, 0, 256, 6);
  x.font = 'bold 30px sans-serif'; x.fillStyle = '#fff';
  x.textAlign = 'center'; x.textBaseline = 'middle'; x.fillText(text.slice(0, 16), 128, 36);
  const spr = new THREE.Sprite(new THREE.SpriteMaterial({
    map: new THREE.CanvasTexture(cv), depthTest: false, transparent: true }));
  spr.scale.set(620, 155, 1); return spr;
}

function buildCars() {
  const map = new Map();
  for (const pl of S.players) {
    const hex = TEAM[pl.team] ?? 0x9aa4b2;
    const g = new THREE.Group();
    const body = new THREE.Mesh(new THREE.BoxGeometry(118, 84, 36),
      new THREE.MeshStandardMaterial({ color: hex, roughness: .5, metalness: .2 }));
    body.position.z = 18; g.add(body);
    const cabin = new THREE.Mesh(new THREE.BoxGeometry(56, 70, 30),
      new THREE.MeshStandardMaterial({ color: hex, roughness: .4 }));
    cabin.position.set(-8, 0, 46); g.add(cabin);
    const nose = new THREE.Mesh(new THREE.BoxGeometry(16, 84, 36),
      new THREE.MeshStandardMaterial({ color: 0xffffff, emissive: 0x444444 }));
    nose.position.set(59, 0, 18); g.add(nose);
    const bar = new THREE.Mesh(new THREE.BoxGeometry(26, 26, 1),
      new THREE.MeshBasicMaterial({ color: 0x22dd66 }));
    g.add(bar);
    const label = makeLabel(pl.name, hex); label.position.set(0, 0, 285); g.add(label);
    scene.add(g);
    const shadow = makeShadow(95);
    const tg = new THREE.BufferGeometry();
    tg.setAttribute('position', new THREE.BufferAttribute(new Float32Array(64 * 3), 3));
    const trail = new THREE.Line(tg, new THREE.LineBasicMaterial({ color: hex, transparent: true, opacity: .55 }));
    trail.frustumCulled = false; scene.add(trail);
    const callout = new THREE.Sprite(new THREE.SpriteMaterial({ transparent: true, depthTest: false, opacity: 0 }));
    callout.scale.set(780, 174, 1); callout.visible = false; scene.add(callout);
    map.set(pl.pri, { g, bar, label, shadow, trail, callout });
  }
  return map;
}

// --- frame lookup + interpolation ---
const frames = S.frames;
const times = frames.map(f => f.t);
function frameIndex(t) {
  if (t <= times[0]) return 0;
  const last = times.length - 1;
  if (t >= times[last]) return last;
  let lo = 0, hi = last;
  while (lo < hi) { const mid = (lo + hi + 1) >> 1; if (times[mid] <= t) lo = mid; else hi = mid - 1; }
  return lo;
}
const lerp3 = (a, b, w) => [a[0] + (b[0] - a[0]) * w, a[1] + (b[1] - a[1]) * w, a[2] + (b[2] - a[2]) * w];
const quatOf = r => new THREE.Quaternion().setFromEuler(new THREE.Euler(r[2], r[0], r[1], 'ZYX'));

function stateAt(t) {
  const i = frameIndex(t), a = frames[i], b = frames[Math.min(i + 1, frames.length - 1)];
  const span = b.t - a.t, w = span > 1e-6 ? Math.min(Math.max((t - a.t) / span, 0), 1) : 0;
  const ball = (a.ball && b.ball) ? lerp3(a.ball, b.ball, w) : (a.ball || b.ball);
  const nb = new Map(b.cars.map(c => [c.pri, c]));
  const out = new Map();
  for (const ca of a.cars) {
    const c2 = nb.get(ca.pri);
    const q = quatOf(ca.rot);
    if (c2) q.slerp(quatOf(c2.rot), w); // smooth spin across frames
    out.set(ca.pri, { p: c2 ? lerp3(ca.p, c2.p, w) : ca.p, q, boost: ca.boost });
  }
  return { ball, cars: out };
}

function trailPoints(pri, t) {
  const i1 = frameIndex(t), t0 = t - 1.3; let i0 = i1;
  while (i0 > 0 && frames[i0].t > t0) i0--;
  const pts = [];
  for (let i = i0; i <= i1; i++) { const c = frames[i].cars.find(c => c.pri === pri); if (c) pts.push(c.p); }
  return pts;
}

// Skill callouts: a fading "★ <skill>" label above the car that just performed
// it (within ~1.4s), so detected skills surface in the 3D scene, not just the
// ticker. Textures are cached per skill name.
const skillEvents = S.events.filter(e => e.kind === 'skill');
const calloutTexCache = new Map();
function calloutTex(label) {
  if (calloutTexCache.has(label)) return calloutTexCache.get(label);
  const cv = document.createElement('canvas'); cv.width = 340; cv.height = 76;
  const x = cv.getContext('2d');
  x.fillStyle = 'rgba(8,32,36,.8)'; x.fillRect(0, 0, 340, 76);
  x.strokeStyle = '#56d4dd'; x.lineWidth = 4; x.strokeRect(2, 2, 336, 72);
  x.font = 'bold 32px sans-serif'; x.fillStyle = '#8af0f7';
  x.textAlign = 'center'; x.textBaseline = 'middle'; x.fillText('★ ' + label.slice(0, 18), 170, 40);
  const t = new THREE.CanvasTexture(cv); calloutTexCache.set(label, t); return t;
}
function updateCallouts(t, st) {
  for (const [pri, o] of cars) {
    let best = null;
    for (const e of skillEvents) { if (e.t > t) break; if (e.pri === pri && t - e.t <= 1.4) best = e; }
    const c = st.cars.get(pri);
    if (best && c && o.callout) {
      const age = t - best.t, name = best.label.split(' — ')[0];
      o.callout.material.map = calloutTex(name);
      o.callout.material.opacity = Math.max(0, 1 - age / 1.4);
      o.callout.position.set(c.p[0], c.p[1], c.p[2] + 360);
      o.callout.visible = true;
    } else if (o.callout) { o.callout.visible = false; }
  }
}

let show = { trails: true, labels: true, boost: true };
function applyState(st) {
  if (st.ball) { ball.visible = true; ball.position.set(...st.ball); ballShadow.visible = true; placeShadow(ballShadow, st.ball[0], st.ball[1], st.ball[2]); }
  else { ball.visible = false; ballShadow.visible = false; }
  for (const [pri, o] of cars) {
    const c = st.cars.get(pri);
    const live = !!c;
    o.g.visible = live; o.shadow.visible = live; o.label.visible = live && show.labels;
    o.bar.visible = live && show.boost; o.trail.visible = live && show.trails;
    if (!live) continue;
    o.g.position.set(...c.p); o.g.quaternion.copy(c.q);
    placeShadow(o.shadow, c.p[0], c.p[1], c.p[2]);
    o.bar.scale.z = Math.max(c.boost * 2.2, 0.5); o.bar.position.z = 72 + c.boost * 1.1;
    o.bar.material.color.setHex(c.boost > 50 ? 0x22dd66 : c.boost > 20 ? 0xe0b020 : 0xdd4030);
    if (show.trails) {
      const pts = trailPoints(pri, T);
      const arr = o.trail.geometry.attributes.position.array;
      const n = Math.min(pts.length, 64);
      for (let i = 0; i < n; i++) { arr[i * 3] = pts[i][0]; arr[i * 3 + 1] = pts[i][1]; arr[i * 3 + 2] = pts[i][2] + 4; }
      o.trail.geometry.setDrawRange(0, n);
      o.trail.geometry.attributes.position.needsUpdate = true;
    }
  }
}

// --- HUD ---
const esc = s => s.replace(/[&<>]/g, m => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;' }[m]));
const fmt = s => { const m = Math.floor(s / 60); return m + ':' + (s % 60).toFixed(1).padStart(4, '0'); };
const dur = S.duration_s;
document.getElementById('scoreboard').innerHTML =
  `<span style="color:#3b82f6">BLUE ${S.team_scores['0'] ?? 0}</span> — ` +
  `<span style="color:#f97316">${S.team_scores['1'] ?? 0} ORANGE</span>`;
document.getElementById('mapline').textContent = (S.map || 'replay') + '  ·  ' + S.replay_id;
const plistEl = document.getElementById('players');
const tickerEl = document.getElementById('ticker');
const possEl = document.getElementById('poss');

// Player rows are built once; only the boost value + follow highlight change.
const rowEls = new Map();
for (const pl of S.players) {
  const row = document.createElement('div');
  row.className = 'row'; row.dataset.pri = pl.pri;
  const css = TEAM_CSS[pl.team] ?? '#9aa4b2';
  row.innerHTML = `<span class="dot" style="background:${css}"></span>${esc(pl.name)}<b>0</b>`;
  row.onclick = () => setCam('player', pl.pri);
  plistEl.appendChild(row);
  rowEls.set(pl.pri, row);
}
function updatePlayers(st) {
  for (const pl of S.players) {
    const row = rowEls.get(pl.pri), c = st ? st.cars.get(pl.pri) : null;
    row.querySelector('b').textContent = c ? c.boost : 0;
    row.classList.toggle('follow', cam.mode === 'player' && cam.pri === pl.pri);
  }
}
function possAt(t) { let team = null; for (const e of S.events) { if (e.t > t + 1e-3) break; if (e.kind === 'touch') team = e.team; } return team; }

// The ticker is rebuilt only when the most-recent-event index changes.
let lastTickerIdx = -2;
function updateHud(t, st) {
  document.getElementById('clock').textContent = fmt(t) + ' / ' + fmt(dur);
  updatePlayers(st);
  const pt = possAt(t);
  possEl.innerHTML = pt == null ? '' : ` · poss <span style="color:${TEAM_CSS[pt] ?? '#888'}">${pt === 0 ? 'BLUE' : 'ORANGE'}</span>`;
  let i = S.events.length - 1;
  while (i >= 0 && S.events[i].t > t + 1e-3) i--;
  if (i !== lastTickerIdx) {
    lastTickerIdx = i;
    const recent = S.events.slice(Math.max(0, i - 6), i + 1);
    tickerEl.innerHTML = recent.map(e =>
      `<div class="ev ${e.kind}" data-t="${e.t}"><span>${fmt(e.t)}</span>${esc(e.label)}</div>`).join('');
    tickerEl.querySelectorAll('.ev').forEach(d => d.onclick = () => seek(+d.dataset.t));
  }
}

// timeline event markers (goals / demos / kickoffs)
const marksEl = document.getElementById('marks');
for (const e of S.events) {
  if (e.kind !== 'goal' && e.kind !== 'demo' && e.kind !== 'kickoff') continue;
  const d = document.createElement('div');
  d.className = 'mk ' + e.kind; d.style.left = (e.t / dur * 100) + '%';
  d.title = fmt(e.t) + '  ' + e.label; d.onclick = () => seek(e.t);
  marksEl.appendChild(d);
}

// --- playback ---
let T = 0, playing = true, speed = 1, loop = false, last = performance.now(), lastState = null;
const tl = document.getElementById('timeline'), playBtn = document.getElementById('play');
tl.max = dur; tl.step = 0.01; playBtn.innerHTML = ICON_PAUSE;
function setPlaying(p) { playing = p; playBtn.innerHTML = p ? ICON_PAUSE : ICON_PLAY; }
function seek(t) { T = Math.min(Math.max(t, 0), dur); tl.value = T; }
playBtn.onclick = () => setPlaying(!playing);
tl.oninput = () => { T = parseFloat(tl.value); setPlaying(false); };
document.getElementById('speed').onchange = e => { speed = parseFloat(e.target.value); };
document.getElementById('loop').onchange = e => { loop = e.target.checked; };
for (const id of ['Trails', 'Labels', 'Boost']) {
  document.getElementById('tg' + id).onchange = e => { show[id.toLowerCase()] = e.target.checked; };
}
function jump(kind, dir) {
  const ts = S.events.filter(e => e.kind === kind).map(e => e.t);
  if (dir > 0) { const n = ts.find(x => x > T + 0.05); if (n != null) { seek(n); setPlaying(false); } }
  else { const p = [...ts].reverse().find(x => x < T - 0.05); if (p != null) { seek(p); setPlaying(false); } }
}
addEventListener('keydown', e => {
  if (e.code === 'Space') { e.preventDefault(); setPlaying(!playing); }
  else if (e.code === 'ArrowRight') seek(T + 1);
  else if (e.code === 'ArrowLeft') seek(T - 1);
  else if (e.key === 'n') jump('goal', 1);
  else if (e.key === 'p') jump('goal', -1);
  else if (e.key === 'k') jump('kickoff', 1);
  else if (e.key === 'j') jump('kickoff', -1);
  else if (e.key === '0') setCam('free');
});

function animate(now) {
  requestAnimationFrame(animate);
  const dt = (now - last) / 1000; last = now;
  if (playing) { T += dt * speed; if (T >= dur) { if (loop) T = 0; else { T = dur; setPlaying(false); } } }
  tl.value = T;
  const st = stateAt(T); lastState = st;
  applyState(st);
  updateHud(T, st);
  updateCallouts(T, st);
  // follow cameras
  let focus = null;
  if (cam.mode === 'ball' && st.ball) focus = new THREE.Vector3(...st.ball);
  else if (cam.mode === 'player' && cam.pri != null) { const c = st.cars.get(cam.pri); if (c) focus = new THREE.Vector3(...c.p); }
  if (focus) {
    controls.target.lerp(focus, 0.18);
    const desired = focus.clone().add(new THREE.Vector3(0, -1500, 750));
    camera.position.lerp(desired, 0.06);
  }
  controls.update();
  renderer.render(scene, camera);
}
requestAnimationFrame(animate);

addEventListener('resize', () => {
  camera.aspect = innerWidth / innerHeight;
  camera.updateProjectionMatrix();
  renderer.setSize(innerWidth, innerHeight);
});
</script>
</body>
</html>
"##;
