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
  #top { top: 10px; left: 50%; transform: translateX(-50%); text-align: center;
    font-size: 16px; font-weight: 700; }
  #top small { display:block; font-weight: 400; font-size: 11px; color:#8b949e; }
  #players { top: 10px; left: 10px; min-width: 150px; }
  #players .dot { display:inline-block; width:9px; height:9px; border-radius:50%;
    margin-right:6px; vertical-align:middle; }
  #players b { float:right; color:#c9d1d9; }
  #ticker { top: 10px; right: 10px; width: 230px; max-height: 46vh; overflow:hidden; }
  #ticker .ev { padding:1px 0; white-space:nowrap; overflow:hidden; text-overflow:ellipsis; }
  #ticker .ev span { color:#6e7681; margin-right:6px; }
  #ticker .goal { color:#ffd166; font-weight:700; }
  #ticker .demo { color:#f97316; }
  #ticker .skill { color:#56d4dd; }
  #ticker .kickoff { color:#a0aec0; }
  #ticker .touch { color:#8b949e; }
  #bar { left: 50%; bottom: 14px; transform: translateX(-50%); display:flex;
    align-items:center; gap:10px; width: min(880px, 92vw); }
  #bar button { background:#21262d; color:#e6edf3; border:1px solid #30363d;
    border-radius:6px; width:34px; height:30px; font-size:15px; cursor:pointer; }
  #bar input[type=range] { flex:1; }
  #bar select { background:#21262d; color:#e6edf3; border:1px solid #30363d;
    border-radius:6px; height:30px; }
  #clock { min-width: 104px; text-align:center; color:#c9d1d9; }
  #help { left:10px; bottom:10px; color:#6e7681; font-size:11px; }
</style>
</head>
<body>
<canvas id="c"></canvas>
<div id="top" class="panel"><span id="scoreboard"></span><small id="mapline"></small></div>
<div id="players" class="panel"></div>
<div id="ticker" class="panel"></div>
<div id="bar" class="panel">
  <button id="play">⏸</button>
  <span id="clock">0:00.0</span>
  <input id="timeline" type="range" min="0" value="0">
  <select id="speed" title="playback speed">
    <option value="0.25">0.25×</option><option value="0.5">0.5×</option>
    <option value="1" selected>1×</option><option value="2">2×</option><option value="4">4×</option>
  </select>
</div>
<div id="help" class="panel">drag to orbit · scroll to zoom · right-drag to pan</div>

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

THREE.Object3D.DEFAULT_UP.set(0, 0, 1); // Rocket League is Z-up

const scene = new THREE.Scene();
scene.background = new THREE.Color(0x0b0e14);
scene.fog = new THREE.Fog(0x0b0e14, F.back_wall_y * 2.2, F.back_wall_y * 4.5);

const camera = new THREE.PerspectiveCamera(55, innerWidth / innerHeight, 10, 60000);
camera.position.set(0, -F.back_wall_y * 1.7, F.ceiling_z * 3.2);

const renderer = new THREE.WebGLRenderer({ antialias: true, canvas: document.getElementById('c') });
renderer.setSize(innerWidth, innerHeight);
renderer.setPixelRatio(Math.min(devicePixelRatio, 2));

const controls = new OrbitControls(camera, renderer.domElement);
controls.target.set(0, 0, 200);
controls.enableDamping = true;
controls.maxDistance = F.back_wall_y * 4;
controls.update();

scene.add(new THREE.HemisphereLight(0xb8d0ff, 0x202830, 1.1));
const dir = new THREE.DirectionalLight(0xffffff, 1.3);
dir.position.set(2500, -3500, 7000);
scene.add(dir);

buildField();
const ball = makeBall();
const cars = buildCars();

function buildField() {
  const fx = F.side_wall_x, fy = F.back_wall_y, fz = F.ceiling_z;
  const floor = new THREE.Mesh(
    new THREE.PlaneGeometry(2 * fx, 2 * fy),
    new THREE.MeshStandardMaterial({ color: 0x12351f, roughness: 1 }));
  scene.add(floor); // PlaneGeometry lies in XY (normal +Z) — a Z-up floor at z=0

  const walls = new THREE.LineSegments(
    new THREE.EdgesGeometry(new THREE.BoxGeometry(2 * fx, 2 * fy, fz)),
    new THREE.LineBasicMaterial({ color: 0x2b4d6f }));
  walls.position.set(0, 0, fz / 2);
  scene.add(walls);

  // halfway line + centre circle
  const lineMat = new THREE.LineBasicMaterial({ color: 0x3a5a78 });
  const mid = new THREE.BufferGeometry().setFromPoints(
    [new THREE.Vector3(-fx, 0, 2), new THREE.Vector3(fx, 0, 2)]);
  scene.add(new THREE.Line(mid, lineMat));
  const circ = new THREE.Mesh(
    new THREE.RingGeometry(900, 920, 48),
    new THREE.MeshBasicMaterial({ color: 0x3a5a78, side: THREE.DoubleSide }));
  circ.position.z = 2;
  scene.add(circ);

  for (const sign of [1, -1]) {
    const g = new THREE.LineSegments(
      new THREE.EdgesGeometry(new THREE.BoxGeometry(2 * F.goal_half_width, 200, F.goal_height)),
      new THREE.LineBasicMaterial({ color: 0xffd166 }));
    g.position.set(0, sign * fy, F.goal_height / 2);
    scene.add(g);
  }
}

function makeBall() {
  const m = new THREE.Mesh(
    new THREE.SphereGeometry(F.ball_radius, 28, 18),
    new THREE.MeshStandardMaterial({ color: 0xeaeaea, emissive: 0x303030, roughness: .4 }));
  scene.add(m);
  return m;
}

function makeLabel(text, hex) {
  const cv = document.createElement('canvas');
  cv.width = 256; cv.height = 64;
  const x = cv.getContext('2d');
  x.fillStyle = 'rgba(0,0,0,.55)'; x.fillRect(0, 0, 256, 64);
  x.fillStyle = '#' + hex.toString(16).padStart(6, '0');
  x.fillRect(0, 0, 256, 6);
  x.font = 'bold 30px sans-serif'; x.fillStyle = '#fff';
  x.textAlign = 'center'; x.textBaseline = 'middle';
  x.fillText(text.slice(0, 16), 128, 36);
  const spr = new THREE.Sprite(new THREE.SpriteMaterial({
    map: new THREE.CanvasTexture(cv), depthTest: false, transparent: true }));
  spr.scale.set(620, 155, 1);
  return spr;
}

function buildCars() {
  const map = new Map();
  for (const pl of S.players) {
    const hex = TEAM[pl.team] ?? 0x9aa4b2;
    const g = new THREE.Group();
    const body = new THREE.Mesh(
      new THREE.BoxGeometry(118, 84, 36),
      new THREE.MeshStandardMaterial({ color: hex, roughness: .5, metalness: .2 }));
    body.position.z = 18;
    g.add(body);
    const nose = new THREE.Mesh( // marks forward (+X)
      new THREE.BoxGeometry(16, 84, 36),
      new THREE.MeshStandardMaterial({ color: 0xffffff }));
    nose.position.set(59, 0, 18);
    g.add(nose);
    const bar = new THREE.Mesh( // boost gauge (unit-height, scaled by boost)
      new THREE.BoxGeometry(20, 20, 1),
      new THREE.MeshBasicMaterial({ color: 0x22dd66 }));
    g.add(bar);
    const label = makeLabel(pl.name, hex);
    label.position.set(0, 0, 280);
    g.add(label);
    g.userData = { bar };
    scene.add(g);
    map.set(pl.pri, g);
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

function stateAt(t) {
  const i = frameIndex(t), a = frames[i], b = frames[Math.min(i + 1, frames.length - 1)];
  const span = b.t - a.t, w = span > 1e-6 ? Math.min(Math.max((t - a.t) / span, 0), 1) : 0;
  let ball = (a.ball && b.ball) ? lerp3(a.ball, b.ball, w) : (a.ball || b.ball);
  const nb = new Map(b.cars.map(c => [c.pri, c]));
  const out = new Map();
  for (const ca of a.cars) {
    const c2 = nb.get(ca.pri);
    out.set(ca.pri, { p: c2 ? lerp3(ca.p, c2.p, w) : ca.p, rot: ca.rot, boost: ca.boost });
  }
  return { ball, cars: out };
}

function applyState(st) {
  if (st.ball) { ball.visible = true; ball.position.set(st.ball[0], st.ball[1], st.ball[2]); }
  else ball.visible = false;
  for (const [pri, g] of cars) {
    const c = st.cars.get(pri);
    if (!c) { g.visible = false; continue; }
    g.visible = true;
    g.position.set(c.p[0], c.p[1], c.p[2]);
    // rot = [pitch, yaw, roll]; apply yaw(Z) -> pitch(Y) -> roll(X).
    g.rotation.set(c.rot[2], c.rot[0], c.rot[1], 'ZYX');
    const bar = g.userData.bar;
    bar.scale.z = Math.max(c.boost * 2, 0.5);
    bar.position.z = 70 + c.boost;
    bar.material.color.setHex(c.boost > 50 ? 0x22dd66 : c.boost > 20 ? 0xe0b020 : 0xdd4030);
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

function updateHud(t, st) {
  document.getElementById('clock').textContent = fmt(t) + ' / ' + fmt(dur);
  plistEl.innerHTML = S.players.map(pl => {
    const c = st.cars.get(pl.pri), boost = c ? c.boost : 0;
    const css = TEAM_CSS[pl.team] ?? '#9aa4b2';
    return `<div><span class="dot" style="background:${css}"></span>${esc(pl.name)}<b>${boost}</b></div>`;
  }).join('');
  let i = S.events.length - 1;
  while (i >= 0 && S.events[i].t > t + 1e-3) i--;
  const recent = S.events.slice(Math.max(0, i - 6), i + 1);
  tickerEl.innerHTML = recent.map(e =>
    `<div class="ev ${e.kind}"><span>${fmt(e.t)}</span>${esc(e.label)}</div>`).join('');
}

// --- playback ---
let t = 0, playing = true, speed = 1, last = performance.now();
const tl = document.getElementById('timeline'), playBtn = document.getElementById('play');
tl.max = dur; tl.step = 0.01;
playBtn.onclick = () => { playing = !playing; playBtn.textContent = playing ? '⏸' : '▶'; };
tl.oninput = () => { t = parseFloat(tl.value); playing = false; playBtn.textContent = '▶'; };
document.getElementById('speed').onchange = e => { speed = parseFloat(e.target.value); };
addEventListener('keydown', e => {
  if (e.code === 'Space') { e.preventDefault(); playBtn.onclick(); }
  else if (e.code === 'ArrowRight') { t = Math.min(dur, t + 1); }
  else if (e.code === 'ArrowLeft') { t = Math.max(0, t - 1); }
});

function animate(now) {
  requestAnimationFrame(animate);
  const dt = (now - last) / 1000; last = now;
  if (playing) { t += dt * speed; if (t >= dur) { t = dur; playing = false; playBtn.textContent = '▶'; } }
  tl.value = t;
  const st = stateAt(t);
  applyState(st);
  updateHud(t, st);
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
