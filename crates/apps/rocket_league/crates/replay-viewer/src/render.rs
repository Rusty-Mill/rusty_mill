//! Self-contained HTML 3D viewer for a [`Scene`].
//!
//! Mirrors `replay_scoring::render::html`: produces one openable `.html` file
//! with the scene data embedded, so the artifact is portable. Rendering uses
//! three.js loaded from a CDN (an import map), so **viewing needs network access**
//! for that one dependency; the match data itself is fully inline.

use crate::scene::Scene;

/// Render a scene to a complete HTML document, loading three.js from a CDN
/// (smaller file; needs network to view).
pub fn html(scene: &Scene) -> String {
    render(scene, CDN_IMPORTS.to_string())
}

/// Like [`html`], but with three.js + OrbitControls embedded as `data:` URLs so
/// the file is fully self-contained and needs **no network** to view. Larger
/// (three.js is ~1.3 MB, base64-encoded into the import map).
pub fn html_offline(scene: &Scene) -> String {
    let three = base64(include_bytes!("../vendor/three.module.js"));
    let orbit = base64(include_bytes!("../vendor/OrbitControls.js"));
    let imports = format!(
        "{{\n  \"three\": \"data:text/javascript;base64,{three}\",\n  \
         \"three/addons/controls/OrbitControls.js\": \"data:text/javascript;base64,{orbit}\"\n}}"
    );
    render(scene, imports)
}

const CDN_IMPORTS: &str = "{\n  \"three\": \
\"https://cdn.jsdelivr.net/npm/three@0.160.0/build/three.module.js\",\n  \
\"three/addons/\": \"https://cdn.jsdelivr.net/npm/three@0.160.0/examples/jsm/\"\n}";

fn render(scene: &Scene, imports: String) -> String {
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
        .replace("/*IMPORTMAP*/", &imports)
}

/// Standard base64 (no line breaks), for embedding vendored JS as data URLs.
fn base64(data: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = ((c[0] as u32) << 16)
            | ((*c.get(1).unwrap_or(&0) as u32) << 8)
            | (*c.get(2).unwrap_or(&0) as u32);
        out.push(A[((n >> 18) & 63) as usize] as char);
        out.push(A[((n >> 12) & 63) as usize] as char);
        out.push(if c.len() > 1 {
            A[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if c.len() > 2 {
            A[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
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
  .panel { position: fixed; z-index: 3; background: rgba(13,17,23,.82); border: 1px solid #1f2937;
    border-radius: 8px; padding: 8px 10px; backdrop-filter: blur(3px); }
  #draw { position: fixed; inset: 0; z-index: 1; pointer-events: none; }
  #tools { top:50%; right:10px; transform:translateY(-50%); display:flex; flex-direction:column;
    gap:5px; align-items:stretch; width:120px; }
  #tools button.on { background:#243049; border-color:#3b82f6; }
  #tools select, #tools input[type=range] { width:100%; }
  #tools label { color:#8b949e; font-size:11px; display:flex; align-items:center; gap:4px; }
  #swatches { display:flex; flex-wrap:wrap; gap:3px; }
  .sw { width:18px; height:18px; border-radius:4px; border:1px solid #30363d; padding:0; cursor:pointer; }
  .sw.on { outline:2px solid #fff; outline-offset:-1px; }
  #helpOverlay { position:fixed; inset:0; z-index:6; display:none; align-items:center; justify-content:center; background:rgba(0,0,0,.55); }
  #helpOverlay .panel { max-width:560px; line-height:1.7; cursor:pointer; }
  #minimap { position:fixed; right:10px; bottom:48px; width:132px; height:165px; padding:0; overflow:hidden; }
  #loading { position:fixed; inset:0; z-index:10; display:flex; flex-direction:column; align-items:center;
    justify-content:center; gap:8px; background:#0b0e14; color:#9aa4b2; font-size:15px; }
  #loading small { color:#6e7681; font-size:11px; }
  button { background:#21262d; color:#e6edf3; border:1px solid #30363d; border-radius:6px;
    cursor:pointer; font:inherit; }
  button:hover { background:#2b333d; }
  #top { top: 10px; left: 50%; transform: translateX(-50%); text-align: center;
    font-size: 16px; font-weight: 700; }
  #top small { display:block; font-weight: 400; font-size: 11px; color:#8b949e; }
  #top #poss { font-size: 11px; font-weight: 700; }
  #warn { top:64px; left:50%; transform:translateX(-50%); font-size:12px; font-weight:700;
    color:#f0b429; background:rgba(46,34,8,.88); border-color:#7a5a10; }
  #players { top: 10px; left: 10px; min-width: 196px; }
  #players .row { cursor:pointer; padding:2px 3px; border-radius:4px; }
  #players .row:hover { background:#1c2230; }
  #players .row.follow { background:#243049; outline:1px solid #3b82f6; }
  #players .r1 { display:flex; align-items:center; }
  #players .dot { display:inline-block; width:9px; height:9px; border-radius:50%; margin-right:6px; }
  #players .r2 { display:flex; align-items:center; gap:8px; font-size:10px; color:#8b949e; padding-left:15px; }
  #players .r2 .bz { margin-left:auto; position:relative; width:56px; height:13px;
    border-radius:7px; background:#21262d; border:1px solid #30363d; overflow:hidden; }
  #players .r2 .bz .f { position:absolute; left:0; top:0; bottom:0; width:0; border-radius:7px; }
  #players .r2 .bz b { position:absolute; inset:0; display:flex; align-items:center;
    justify-content:center; font-size:9px; line-height:1; color:#e6edf3; text-shadow:0 0 2px rgba(0,0,0,.9); }
  #players .r2 .sp { color:#6e7681; }
  #players .role { font-size:9px; color:#6e7681; margin-left:6px; }
  #players .role.first { color:#ffd166; }
  #players .imp { margin-left:auto; font-size:10px; font-weight:700; cursor:help; }
  #ticker .ev .d { font-weight:700; }
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
  #left .ov { display:flex; align-items:center; gap:5px; color:#8b949e; }
  #left .ov select, #left .ov button { height:24px; padding:0 6px; }
  #left .ov input[type=range] { width:74px; }
  #help { left:10px; bottom:10px; color:#6e7681; font-size:11px; }
  #bar { left: 50%; bottom: 14px; transform: translateX(-50%); display:flex;
    align-items:center; gap:10px; width: min(900px, 92vw); }
  #bar button.icon { width:34px; height:30px; display:flex; align-items:center; justify-content:center; }
  #bar select { background:#21262d; color:#e6edf3; border:1px solid #30363d; border-radius:6px; height:30px; }
  #clock { min-width: 116px; text-align:center; color:#c9d1d9; }
  #tlwrap { position:relative; flex:1; display:flex; flex-direction:column; gap:1px; }
  #wp { width:100%; height:20px; display:block; border-radius:3px; }
  #pressure { width:100%; height:14px; display:block; border-radius:3px; margin-bottom:2px; }
  #wpcursor { position:absolute; top:0; height:20px; width:1px; background:#fff; opacity:.7; pointer-events:none; }
  #tltrack { position:relative; height:24px; display:flex; align-items:center; }
  #timeline { width:100%; }
  #marks { position:absolute; left:0; right:0; top:5px; height:10px; pointer-events:none; }
  #loopRegion { position:absolute; top:2px; height:20px; display:none; pointer-events:none;
    background:rgba(86,212,221,.16); border-left:2px solid #56d4dd; border-right:2px solid #56d4dd; }
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
<canvas id="draw"></canvas>
<div id="loading">Loading replay…<small>(first load fetches three.js — needs network unless built with --offline)</small></div>
<div id="tools" class="panel">
  <button id="drawToggle" title="draw mode (d)">draw</button>
  <div id="swatches"></div>
  <select id="drawTool" title="tool"><option value="pen">pen</option><option value="arrow">arrow</option><option value="line">line</option></select>
  <label>w<input type="range" id="drawWidth" min="2" max="16" value="5"></label>
  <label><input type="checkbox" id="draw3d"> on field</label>
  <button id="drawUndo" title="undo (z)">undo</button>
  <button id="drawClear">clear</button>
  <button id="drawSave" title="save PNG (s)">save png</button>
</div>
<div id="helpOverlay"><div class="panel">
  <b>Keyboard</b><br>
  space play/pause · ◀ ▶ ±1s · n / p next·prev goal · k / j next·prev kickoff<br>
  i / o set an A–B loop · x clear it · 0 free cam · d draw · z undo · s save PNG · ? help<br><br>
  <b>Mouse</b><br>
  drag orbit · scroll zoom · right-drag pan · click a player row to follow it<br><br>
  <b>Tools</b><br>
  overlay: thirds / lanes / grid, or drop an image on the field · telestrator: arrows / lines / pen · heatmap toggle<br><br>
  <span style="color:#6e7681">press ? or click to close</span>
</div></div>
<div id="top" class="panel"><span id="scoreboard"></span><small id="mapline"></small><span id="poss"></span></div>
<div id="warn" class="panel" style="display:none">⚠ non-standard map — drawn field &amp; positional overlays are approximate</div>
<div id="players" class="panel"></div>
<div id="ticker" class="panel"></div>
<div id="left">
  <div class="panel cams">
    <button data-cam="free" class="on">overview</button>
    <button data-cam="goal">goal</button>
    <button data-cam="ball">ball</button>
    <button data-cam="broadcast">tv</button>
  </div>
  <div class="panel">
    <label><input type="checkbox" id="tgTrails" checked> trails</label>
    <label><input type="checkbox" id="tgLabels" checked> labels</label>
    <label><input type="checkbox" id="tgBoost" checked> boost</label>
    <label><input type="checkbox" id="tgPads" checked> pads</label>
    <label><input type="checkbox" id="tgHeat"> heatmap</label>
  </div>
  <div class="panel ov">
    overlay
    <select id="ovSel" title="field overlay">
      <option value="none">none</option>
      <option value="thirds">thirds</option>
      <option value="lanes">lanes</option>
      <option value="grid">grid</option>
      <option value="image" id="ovImgOpt" disabled>image</option>
    </select>
    <button id="ovLoad" title="load a field image (or drag one onto the view)">img…</button>
    <input type="file" id="ovFile" accept="image/*" style="display:none">
    <input type="range" id="ovOpacity" min="0" max="100" value="55" title="overlay opacity">
  </div>
</div>
<canvas id="minimap" class="panel"></canvas>
<div id="help" class="panel">space play · ◀▶ ±1s · n/p goal · k/j kickoff · click player to follow · d draw · s png · ? help</div>
<div id="bar" class="panel">
  <button id="play" class="icon" title="play/pause (space)"></button>
  <span id="clock">0:00.0</span>
  <div id="tlwrap">
    <canvas id="pressure" title="pressure — which half the ball is in: blue pressing above, orange below"></canvas>
    <canvas id="wp" title="momentum — P(next goal): blue above, orange below"></canvas>
    <div id="wpcursor"></div>
    <div id="tltrack"><div id="loopRegion"></div><input id="timeline" type="range" min="0" value="0"><div id="marks"></div></div>
  </div>
  <select id="speed" title="playback speed">
    <option value="0.25">0.25×</option><option value="0.5">0.5×</option>
    <option value="1" selected>1×</option><option value="2">2×</option><option value="4">4×</option>
  </select>
  <label><input type="checkbox" id="loop"> loop</label>
</div>

<script type="importmap">
{ "imports": /*IMPORTMAP*/ }
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
scene.background = gradientBg();
scene.fog = new THREE.Fog(0x0b0e14, F.back_wall_y * 2.6, F.back_wall_y * 5.5);

const camera = new THREE.PerspectiveCamera(55, innerWidth / innerHeight, 10, 60000);
const renderer = new THREE.WebGLRenderer({ antialias: true, canvas: document.getElementById('c'), preserveDrawingBuffer: true });
renderer.setSize(innerWidth, innerHeight);
renderer.setPixelRatio(Math.min(devicePixelRatio, 2));
renderer.toneMapping = THREE.ACESFilmicToneMapping;
renderer.toneMappingExposure = 1.1;
renderer.shadowMap.enabled = true;
renderer.shadowMap.type = THREE.PCFSoftShadowMap;

const controls = new OrbitControls(camera, renderer.domElement);
controls.enableDamping = true;
controls.maxDistance = F.back_wall_y * 4;

scene.add(new THREE.HemisphereLight(0xbcd3ff, 0x1a2230, 1.0));
const dir = new THREE.DirectionalLight(0xffffff, 2.2);
dir.position.set(2800, -3200, 7000);
dir.castShadow = true;
dir.shadow.mapSize.set(2048, 2048);
dir.shadow.bias = -0.0004;
Object.assign(dir.shadow.camera, { near: 800, far: 20000, left: -6500, right: 6500, top: 7500, bottom: -7500 });
dir.shadow.camera.updateProjectionMatrix();
scene.add(dir);

function gradientBg() {
  const cv = document.createElement('canvas'); cv.width = 2; cv.height = 256;
  const x = cv.getContext('2d'), g = x.createLinearGradient(0, 0, 0, 256);
  g.addColorStop(0, '#10161f'); g.addColorStop(0.55, '#0b0e14'); g.addColorStop(1, '#05070a');
  x.fillStyle = g; x.fillRect(0, 0, 2, 256);
  const t = new THREE.CanvasTexture(cv); t.colorSpace = THREE.SRGBColorSpace; return t;
}

// --- camera presets ---
const OVERVIEW = { pos: new THREE.Vector3(0, -F.back_wall_y * 1.22, F.ceiling_z * 2.0), tgt: new THREE.Vector3(0, 0, 150) };
const GOALVIEW = { pos: new THREE.Vector3(0, -F.back_wall_y * 1.5, F.ceiling_z * 1.1), tgt: new THREE.Vector3(0, F.back_wall_y * 0.3, 200) };
const BROADCAST_POS = new THREE.Vector3(F.side_wall_x * 1.4, -F.back_wall_y * 0.2, F.ceiling_z * 1.6);
let cam = { mode: 'free', pri: null };
let camTween = null; // eased preset transition
function applyPose(p, snap) {
  if (snap) { camera.position.copy(p.pos); controls.target.copy(p.tgt); controls.update(); camTween = null; return; }
  camTween = { fromPos: camera.position.clone(), toPos: p.pos.clone(), fromTgt: controls.target.clone(), toTgt: p.tgt.clone(), start: performance.now(), dur: 650 };
}
applyPose(OVERVIEW, true);
controls.addEventListener('start', () => camTween = null); // user grab cancels the tween

function setCam(mode, pri) {
  cam = { mode, pri: pri ?? null };
  document.querySelectorAll('#left .cams button').forEach(b => b.classList.toggle('on', b.dataset.cam === mode));
  if (mode === 'free') applyPose(OVERVIEW);
  else if (mode === 'goal') applyPose(GOALVIEW);
  if (mode === 'player') heatSubject = pri;
  else if (mode === 'ball') heatSubject = 'ball';
  refreshHeat(); // re-bin for the new subject if the heatmap is shown
  updatePlayers(lastState); // refresh follow highlight
}
document.querySelectorAll('#left .cams button').forEach(b => b.onclick = () => setCam(b.dataset.cam));

buildField();
const ball = makeBall();
const ballTrail = new THREE.Line(
  new THREE.BufferGeometry().setAttribute('position', new THREE.BufferAttribute(new Float32Array(64 * 3), 3)),
  new THREE.LineBasicMaterial({ color: 0xe6e6e6, transparent: true, opacity: .5 }));
ballTrail.frustumCulled = false; scene.add(ballTrail);
const cars = buildCars();

// --- floor heatmap: occupancy of the ball or the followed car over the match,
// computed from the same grid positions the scoring heatmaps use ---
const heatMesh = new THREE.Mesh(
  new THREE.PlaneGeometry(2 * F.side_wall_x, 2 * F.back_wall_y),
  new THREE.MeshBasicMaterial({ transparent: true, opacity: .72, depthWrite: false }));
heatMesh.position.z = 6; heatMesh.visible = false; scene.add(heatMesh);
let heatSubject = 'ball';
function heatColor(t) { // blue → cyan → green → yellow → red
  const stops = [[30, 60, 160], [40, 200, 210], [80, 210, 90], [240, 210, 60], [230, 60, 40]];
  const s = Math.min(0.999, Math.max(0, t)) * (stops.length - 1);
  const i = Math.floor(s), f = s - i, a = stops[i], b = stops[i + 1];
  return [a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f, a[2] + (b[2] - a[2]) * f];
}
function buildHeat(subject) {
  const NX = 48, NY = 60, fx = F.side_wall_x, fy = F.back_wall_y;
  const grid = new Float32Array(NX * NY);
  for (const f of frames) {
    const p = subject === 'ball' ? f.ball : (f.cars.find(c => c.pri === subject)?.p || null);
    if (!p) continue;
    const gx = Math.min(NX - 1, Math.max(0, Math.floor((p[0] + fx) / (2 * fx) * NX)));
    const gy = Math.min(NY - 1, Math.max(0, Math.floor((p[1] + fy) / (2 * fy) * NY)));
    grid[gy * NX + gx]++;
  }
  let max = 0; for (const v of grid) max = Math.max(max, v);
  const cv = document.createElement('canvas'); cv.width = NX; cv.height = NY;
  const ctx = cv.getContext('2d'), img = ctx.createImageData(NX, NY);
  for (let gy = 0; gy < NY; gy++) for (let gx = 0; gx < NX; gx++) {
    const v = grid[gy * NX + gx], t = max > 0 ? Math.pow(v / max, 0.6) : 0;
    const di = ((NY - 1 - gy) * NX + gx) * 4; // canvas row 0 is +y far end
    const [r, g, b] = heatColor(t);
    img.data[di] = r; img.data[di + 1] = g; img.data[di + 2] = b;
    img.data[di + 3] = t > 0.12 ? Math.min(230, t * 255) : 0; // hide sparse cells
  }
  ctx.putImageData(img, 0, 0);
  const tex = new THREE.CanvasTexture(cv); tex.minFilter = THREE.LinearFilter;
  heatMesh.material.map = tex; heatMesh.material.needsUpdate = true;
}
function refreshHeat() { if (heatMesh.visible) buildHeat(heatSubject); }

// --- field overlay: tactical zone presets, or a loaded/dropped image, projected
// on the floor to break down spaces ---
const overlayMesh = new THREE.Mesh(
  new THREE.PlaneGeometry(2 * F.side_wall_x, 2 * F.back_wall_y),
  new THREE.MeshBasicMaterial({ transparent: true, opacity: .55, depthWrite: false }));
overlayMesh.position.z = 4; overlayMesh.visible = false; scene.add(overlayMesh);
let overlayImageTex = null;
function presetTexture(kind) {
  const W = 1024, H = 1280, fx = F.side_wall_x, fy = F.back_wall_y;
  const cv = document.createElement('canvas'); cv.width = W; cv.height = H;
  const x = cv.getContext('2d');
  const X = wx => (wx + fx) / (2 * fx) * W, Y = wy => (fy - wy) / (2 * fy) * H;
  x.lineWidth = 3; x.font = 'bold 34px sans-serif'; x.textAlign = 'center';
  if (kind === 'thirds' || kind === 'grid') {
    const t = fy / 3;
    x.fillStyle = 'rgba(59,130,246,.16)'; x.fillRect(0, Y(-t), W, Y(-fy) - Y(-t));
    x.fillStyle = 'rgba(249,115,22,.16)'; x.fillRect(0, Y(fy), W, Y(t) - Y(fy));
    x.strokeStyle = 'rgba(255,255,255,.5)';
    x.beginPath(); x.moveTo(0, Y(-t)); x.lineTo(W, Y(-t)); x.moveTo(0, Y(t)); x.lineTo(W, Y(t)); x.stroke();
    x.fillStyle = 'rgba(255,255,255,.65)';
    x.fillText('DEFENSIVE', W / 2, Y(-fy * 0.66)); x.fillText('MIDFIELD', W / 2, Y(0) - 10); x.fillText('ATTACKING', W / 2, Y(fy * 0.66));
  }
  if (kind === 'lanes' || kind === 'grid') {
    const c = fx / 3;
    x.strokeStyle = 'rgba(255,255,255,.5)';
    x.beginPath(); x.moveTo(X(-c), 0); x.lineTo(X(-c), H); x.moveTo(X(c), 0); x.lineTo(X(c), H); x.stroke();
    if (kind === 'lanes') { x.fillStyle = 'rgba(255,255,255,.6)'; x.fillText('LEFT', X(-fx * 0.66), 64); x.fillText('CENTER', X(0), 64); x.fillText('RIGHT', X(fx * 0.66), 64); }
  }
  x.strokeStyle = 'rgba(255,255,255,.6)'; x.beginPath(); x.moveTo(0, Y(0)); x.lineTo(W, Y(0)); x.stroke();
  const tex = new THREE.CanvasTexture(cv); tex.minFilter = THREE.LinearFilter; return tex;
}
function setOverlay(kind) {
  if (kind === 'none' || (kind === 'image' && !overlayImageTex)) { overlayMesh.visible = false; return; }
  overlayMesh.material.map = kind === 'image' ? overlayImageTex : presetTexture(kind);
  overlayMesh.material.needsUpdate = true; overlayMesh.visible = true;
}
document.getElementById('ovSel').onchange = e => setOverlay(e.target.value);
document.getElementById('ovOpacity').oninput = e => { overlayMesh.material.opacity = e.target.value / 100; };
function loadOverlayImage(file) {
  const url = URL.createObjectURL(file), img = new Image();
  img.onload = () => {
    overlayImageTex = new THREE.Texture(img); overlayImageTex.colorSpace = THREE.SRGBColorSpace; overlayImageTex.needsUpdate = true;
    document.getElementById('ovImgOpt').disabled = false;
    document.getElementById('ovSel').value = 'image'; setOverlay('image');
    URL.revokeObjectURL(url);
  };
  img.src = url;
}
document.getElementById('ovLoad').onclick = () => document.getElementById('ovFile').click();
document.getElementById('ovFile').onchange = e => { if (e.target.files[0]) loadOverlayImage(e.target.files[0]); };
addEventListener('dragover', e => e.preventDefault());
addEventListener('drop', e => { e.preventDefault(); const f = [...e.dataTransfer.files].find(f => f.type.startsWith('image/')); if (f) loadOverlayImage(f); });

// boost pads as faint field markers: 6 big (rings) + the 28 standard small (dots)
const BIG_PADS = [[3072, 4096], [-3072, 4096], [3072, -4096], [-3072, -4096], [3584, 0], [-3584, 0]];
const SMALL_PADS = [
  [0, -4240], [-1792, -4184], [1792, -4184], [-940, -3308], [940, -3308], [0, -2816],
  [-3584, -2484], [3584, -2484], [-1788, -2300], [1788, -2300], [-2048, -1036], [0, -1024],
  [2048, -1036], [-1024, 0], [1024, 0], [-2048, 1036], [0, 1024], [2048, 1036],
  [-1788, 2300], [1788, 2300], [-3584, 2484], [3584, 2484], [0, 2816], [-940, 3308],
  [940, 3308], [-1792, 4184], [1792, 4184], [0, 4240],
];
const bigPadMat = new THREE.MeshBasicMaterial({ color: 0xffcf5a, transparent: true, opacity: .45, side: THREE.DoubleSide });
const smallPadMat = new THREE.MeshBasicMaterial({ color: 0xe0b840, transparent: true, opacity: .35, side: THREE.DoubleSide });
const bigPadGeo = new THREE.RingGeometry(140, 165, 24), smallPadGeo = new THREE.CircleGeometry(34, 16);
// Per-pad meshes (own material clone) keyed by position, so the pickup-map
// overlay can flash a single pad when it's collected.
function padKey(x, y) { return Math.round(x) + ',' + Math.round(y); }
const padByKey = new Map();
for (const [px, py] of BIG_PADS) {
  const m = new THREE.Mesh(bigPadGeo, bigPadMat.clone()); m.position.set(px, py, 3); scene.add(m);
  padByKey.set(padKey(px, py), { mesh: m, base: 0xffcf5a, baseOp: .45 });
}
for (const [px, py] of SMALL_PADS) {
  const m = new THREE.Mesh(smallPadGeo, smallPadMat.clone()); m.position.set(px, py, 3); scene.add(m);
  padByKey.set(padKey(px, py), { mesh: m, base: 0xe0b840, baseOp: .35 });
}
// Pickup-map: each frame, flash pads collected in the trailing window, tinted by
// the collecting team. Recomputed from the playhead, so scrubbing just works.
const PAD_FLASH_S = 1.2;
const padPickups = S.pad_pickups || [];
function updatePads(t) {
  for (const p of padByKey.values()) {
    p.mesh.material.color.setHex(p.base); p.mesh.material.opacity = p.baseOp; p.mesh.scale.setScalar(1);
  }
  if (!show.pads) return;
  for (const pk of padPickups) {            // sorted by t; later (more recent) wins
    const age = t - pk.t;
    if (age < 0) break;
    if (age > PAD_FLASH_S) continue;
    const e = padByKey.get(padKey(pk.pad[0], pk.pad[1])); if (!e) continue;
    const k = 1 - age / PAD_FLASH_S;        // 1 at pickup → 0 at window end
    e.mesh.material.color.setHex(pk.team === 0 ? 0x3b82f6 : pk.team === 1 ? 0xf97316 : 0xffffff);
    e.mesh.material.opacity = Math.min(1, e.baseOp + k * (pk.stolen ? 0.95 : 0.7));
    e.mesh.scale.setScalar(1 + k * (pk.big ? 0.5 : 0.3));
  }
}

// --- telestrator: freehand / arrow / line drawing over the view (a coach's pen).
// Strokes persist on screen until cleared; drawing disables orbit and pauses. ---
const drawCv = document.getElementById('draw'), dctx = drawCv.getContext('2d');
let drawMode = false, drawingNow = false, strokes = [], curStroke = null;
const pen = { color: '#ffd166', width: 5, tool: 'pen', field: false };
function sizeDraw() {
  const dpr = Math.min(devicePixelRatio, 2);
  drawCv.width = innerWidth * dpr; drawCv.height = innerHeight * dpr;
  drawCv.style.width = innerWidth + 'px'; drawCv.style.height = innerHeight + 'px';
  dctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  redrawStrokes();
}
function strokePath(s) {
  if (!s.points.length) return;
  dctx.strokeStyle = s.color; dctx.lineWidth = s.width; dctx.lineCap = 'round'; dctx.lineJoin = 'round';
  if (s.tool === 'pen') {
    dctx.beginPath(); dctx.moveTo(s.points[0][0], s.points[0][1]);
    for (let i = 1; i < s.points.length; i++) dctx.lineTo(s.points[i][0], s.points[i][1]);
    dctx.stroke();
  } else {
    const a = s.points[0], b = s.points[s.points.length - 1];
    dctx.beginPath(); dctx.moveTo(a[0], a[1]); dctx.lineTo(b[0], b[1]); dctx.stroke();
    if (s.tool === 'arrow') {
      const ang = Math.atan2(b[1] - a[1], b[0] - a[0]), h = 9 + s.width * 2.2;
      dctx.beginPath();
      dctx.moveTo(b[0], b[1]); dctx.lineTo(b[0] - h * Math.cos(ang - .4), b[1] - h * Math.sin(ang - .4));
      dctx.moveTo(b[0], b[1]); dctx.lineTo(b[0] - h * Math.cos(ang + .4), b[1] - h * Math.sin(ang + .4));
      dctx.stroke();
    }
  }
}
function redrawStrokes() {
  dctx.clearRect(0, 0, innerWidth, innerHeight);
  for (const s of strokes) strokePath(s);
  if (curStroke) strokePath(curStroke);
}
function setDrawMode(on) {
  drawMode = on;
  drawCv.style.pointerEvents = on ? 'auto' : 'none';
  drawCv.style.cursor = on ? 'crosshair' : '';
  controls.enabled = !on;
  document.getElementById('drawToggle').classList.toggle('on', on);
  if (on) setPlaying(false);
}
// world-anchored (on-field) drawing: raycast the pointer to the floor plane and
// build a 3D polyline that stays put as the camera moves.
const fieldGroup = new THREE.Group(); scene.add(fieldGroup);
const raycaster = new THREE.Raycaster();
const floorPlane = new THREE.Plane(new THREE.Vector3(0, 0, 1), 0);
const undoStack = []; // each entry removes one stroke (screen or field)
let fieldStroke = null;
function fieldPoint(e) {
  raycaster.setFromCamera(new THREE.Vector2((e.clientX / innerWidth) * 2 - 1, -(e.clientY / innerHeight) * 2 + 1), camera);
  const p = new THREE.Vector3();
  return raycaster.ray.intersectPlane(floorPlane, p) ? p.setZ(7) : null;
}
function pushFieldPoint(p) {
  const i = fieldStroke.n;
  if (i * 3 + 2 >= fieldStroke.arr.length) return;
  fieldStroke.arr.set([p.x, p.y, p.z], i * 3); fieldStroke.n++;
  fieldStroke.line.geometry.setDrawRange(0, fieldStroke.n);
  fieldStroke.line.geometry.attributes.position.needsUpdate = true;
}
drawCv.addEventListener('pointerdown', e => {
  if (!drawMode) return;
  drawingNow = true;
  try { drawCv.setPointerCapture(e.pointerId); } catch (_) { /* synthetic events */ }
  if (pen.field) {
    const p = fieldPoint(e); if (!p) { drawingNow = false; return; }
    const arr = new Float32Array(3 * 2048), geom = new THREE.BufferGeometry();
    geom.setAttribute('position', new THREE.BufferAttribute(arr, 3));
    const line = new THREE.Line(geom, new THREE.LineBasicMaterial({ color: pen.color }));
    line.frustumCulled = false; fieldGroup.add(line);
    fieldStroke = { line, arr, n: 0 }; pushFieldPoint(p);
  } else {
    curStroke = { color: pen.color, width: pen.width, tool: pen.tool, points: [[e.clientX, e.clientY]] };
  }
});
drawCv.addEventListener('pointermove', e => {
  if (!drawingNow) return;
  if (fieldStroke) { const p = fieldPoint(e); if (p) pushFieldPoint(p); }
  else if (curStroke) {
    if (pen.tool === 'pen') curStroke.points.push([e.clientX, e.clientY]);
    else curStroke.points[1] = [e.clientX, e.clientY];
    redrawStrokes();
  }
});
addEventListener('pointerup', () => {
  if (drawingNow) {
    if (fieldStroke) { const fs = fieldStroke; undoStack.push(() => { fieldGroup.remove(fs.line); fs.line.geometry.dispose(); }); fieldStroke = null; }
    else if (curStroke) { const s = curStroke; strokes.push(s); undoStack.push(() => { const i = strokes.indexOf(s); if (i >= 0) strokes.splice(i, 1); redrawStrokes(); }); curStroke = null; }
  }
  drawingNow = false; redrawStrokes();
});
function undoLast() { const f = undoStack.pop(); if (f) f(); }
function clearAll() { while (undoStack.length) undoStack.pop()(); }
document.getElementById('drawToggle').onclick = () => setDrawMode(!drawMode);
document.getElementById('drawTool').onchange = e => pen.tool = e.target.value;
document.getElementById('drawWidth').oninput = e => pen.width = +e.target.value;
document.getElementById('draw3d').onchange = e => pen.field = e.target.checked;
document.getElementById('drawUndo').onclick = undoLast;
document.getElementById('drawClear').onclick = clearAll;
const swatchEl = document.getElementById('swatches');
for (const c of ['#ffd166', '#ff5d5d', '#3b82f6', '#22dd66', '#ffffff', '#0b0e14']) {
  const b = document.createElement('button'); b.className = 'sw'; b.style.background = c;
  b.onclick = () => { pen.color = c; swatchEl.querySelectorAll('.sw').forEach(x => x.classList.remove('on')); b.classList.add('on'); };
  swatchEl.appendChild(b);
}
swatchEl.firstChild.classList.add('on');
sizeDraw();

// screenshot: composite the 3D view + any drawings into a PNG download
function exportPNG() {
  renderer.render(scene, camera);
  const out = document.createElement('canvas'); out.width = innerWidth; out.height = innerHeight;
  const x = out.getContext('2d');
  x.drawImage(renderer.domElement, 0, 0, innerWidth, innerHeight);
  if (strokes.length || curStroke) x.drawImage(drawCv, 0, 0, innerWidth, innerHeight);
  const a = document.createElement('a');
  a.download = (S.replay_id || 'replay') + '-' + Math.round(T) + 's.png';
  a.href = out.toDataURL('image/png'); a.click();
}
document.getElementById('drawSave').onclick = exportPNG;
const helpEl = document.getElementById('helpOverlay');
helpEl.onclick = () => { helpEl.style.display = 'none'; };

// minimap: top-down field with live car (team-coloured, 1st-man ringed) + ball dots
const mm = document.getElementById('minimap'), mmx = mm.getContext('2d');
const MMW = 132, MMH = 165;
mm.width = MMW * 2; mm.height = MMH * 2; mmx.scale(2, 2);
function drawMinimap(st) {
  mmx.clearRect(0, 0, MMW, MMH);
  mmx.fillStyle = '#10301a'; mmx.fillRect(0, 0, MMW, MMH);
  const fx = F.side_wall_x, fy = F.back_wall_y;
  const MX = wx => (wx + fx) / (2 * fx) * MMW, MY = wy => (fy - wy) / (2 * fy) * MMH;
  mmx.strokeStyle = 'rgba(255,255,255,.35)'; mmx.lineWidth = 1;
  mmx.strokeRect(2, 2, MMW - 4, MMH - 4);
  mmx.beginPath(); mmx.moveTo(0, MY(0)); mmx.lineTo(MMW, MY(0)); mmx.stroke();
  mmx.beginPath(); mmx.arc(MX(0), MY(0), 11, 0, Math.PI * 2); mmx.stroke();
  for (const pl of S.players) {
    const c = st.cars.get(pl.pri); if (!c) continue;
    mmx.fillStyle = TEAM_CSS[pl.team] ?? '#9aa4b2';
    mmx.beginPath(); mmx.arc(MX(c.p[0]), MY(c.p[1]), c.role === 1 ? 4 : 3, 0, Math.PI * 2); mmx.fill();
    if (c.role === 1) { mmx.strokeStyle = '#ffd166'; mmx.lineWidth = 1.5; mmx.stroke(); }
  }
  if (st.ball) { mmx.fillStyle = '#fff'; mmx.beginPath(); mmx.arc(MX(st.ball[0]), MY(st.ball[1]), 3, 0, Math.PI * 2); mmx.fill(); }
}

// demo flashes: an expanding burst at the demoer when a demo event is crossed
const demoEvents = S.events.filter(e => e.kind === 'demo');
const flashes = [];
let prevT = 0;
function spawnFlash(p) {
  const m = new THREE.Mesh(new THREE.RingGeometry(20, 60, 24),
    new THREE.MeshBasicMaterial({ color: 0xff5533, transparent: true, side: THREE.DoubleSide, depthWrite: false }));
  m.position.set(p[0], p[1], 45); scene.add(m); flashes.push({ m, start: performance.now() });
}
function updateFlashes(now) {
  for (let i = flashes.length - 1; i >= 0; i--) {
    const f = flashes[i], age = (now - f.start) / 700;
    if (age >= 1) { scene.remove(f.m); f.m.material.dispose(); f.m.geometry.dispose(); flashes.splice(i, 1); continue; }
    const s = 1 + age * 5; f.m.scale.set(s, s, 1); f.m.material.opacity = 0.85 * (1 - age);
  }
}

function buildField() {
  const fx = F.side_wall_x, fy = F.back_wall_y, fz = F.ceiling_z;
  const floor = new THREE.Mesh(
    new THREE.PlaneGeometry(2 * fx, 2 * fy),
    new THREE.MeshStandardMaterial({ map: pitchTexture(), roughness: 1, metalness: 0 }));
  floor.receiveShadow = true;
  scene.add(floor);
  const walls = new THREE.LineSegments(
    new THREE.EdgesGeometry(new THREE.BoxGeometry(2 * fx, 2 * fy, fz)),
    new THREE.LineBasicMaterial({ color: 0x2b4d6f, transparent: true, opacity: .6 }));
  walls.position.set(0, 0, fz / 2);
  scene.add(walls);
  buildGoal(1); buildGoal(-1);
}

// Painted pitch: green base + mow stripes, boundary, halfway line, centre circle,
// and goal areas — baked once into a floor texture.
function pitchTexture() {
  const W = 1024, H = 1280, fx = F.side_wall_x, fy = F.back_wall_y;
  const cv = document.createElement('canvas'); cv.width = W; cv.height = H;
  const x = cv.getContext('2d');
  x.fillStyle = '#16401f'; x.fillRect(0, 0, W, H);
  for (let i = 0; i < 12; i++) { x.fillStyle = i % 2 ? 'rgba(255,255,255,.03)' : 'rgba(0,0,0,.035)'; x.fillRect(0, i * H / 12, W, H / 12); }
  const X = wx => (wx + fx) / (2 * fx) * W, Y = wy => (fy - wy) / (2 * fy) * H;
  x.strokeStyle = 'rgba(225,238,228,.55)'; x.lineWidth = 4;
  x.strokeRect(8, 8, W - 16, H - 16);
  x.beginPath(); x.moveTo(0, Y(0)); x.lineTo(W, Y(0)); x.stroke();
  x.beginPath(); x.arc(X(0), Y(0), 920 / (2 * fx) * W, 0, Math.PI * 2); x.stroke();
  for (const s of [1, -1]) {
    const gw = 1300, gd = 1300;
    x.strokeRect(X(-gw), Y(s * fy), X(gw) - X(-gw), Y(s * (fy - gd)) - Y(s * fy));
  }
  const t = new THREE.CanvasTexture(cv); t.colorSpace = THREE.SRGBColorSpace; t.anisotropy = 8; return t;
}

function netTexture() {
  const cv = document.createElement('canvas'); cv.width = 64; cv.height = 64;
  const x = cv.getContext('2d'); x.strokeStyle = 'rgba(255,255,255,.85)'; x.lineWidth = 3;
  for (let i = 0; i <= 64; i += 10) { x.beginPath(); x.moveTo(i, 0); x.lineTo(i, 64); x.moveTo(0, i); x.lineTo(64, i); x.stroke(); }
  const t = new THREE.CanvasTexture(cv); t.wrapS = t.wrapT = THREE.RepeatWrapping; return t;
}

// A goal: frame (posts + crossbar) + a translucent net, tinted by the team that
// defends this end (it attacks the opposite goal), so attack direction reads.
function buildGoal(sign) {
  const gw = F.goal_half_width, gh = F.goal_height, fy = F.back_wall_y, depth = 330;
  const defEntry = Object.entries(S.attack_sign || {}).find(([, s]) => s === -sign);
  const teamHex = defEntry ? (TEAM[+defEntry[0]] ?? null) : null;
  const frameCol = teamHex != null ? new THREE.Color(teamHex).lerp(new THREE.Color(0xffffff), 0.55) : new THREE.Color(0xe8e8e8);
  const netCol = teamHex != null ? new THREE.Color(teamHex).lerp(new THREE.Color(0xffffff), 0.35) : new THREE.Color(0xffffff);
  const frame = new THREE.MeshStandardMaterial({ color: frameCol, roughness: .7 });
  const post = px => { const m = new THREE.Mesh(new THREE.BoxGeometry(22, 22, gh + 22), frame); m.position.set(px, sign * fy, gh / 2); m.castShadow = true; scene.add(m); };
  post(-gw); post(gw);
  const cross = new THREE.Mesh(new THREE.BoxGeometry(2 * gw + 22, 22, 22), frame);
  cross.position.set(0, sign * fy, gh + 11); cross.castShadow = true; scene.add(cross);
  const netMat = () => new THREE.MeshBasicMaterial({ map: netTexture(), color: netCol, transparent: true, opacity: .42, side: THREE.DoubleSide, depthWrite: false });
  const back = new THREE.Mesh(new THREE.PlaneGeometry(2 * gw, gh), netMat());
  back.rotation.x = -Math.PI / 2; back.position.set(0, sign * (fy + depth), gh / 2);
  back.material.map.repeat.set(6, 3); scene.add(back);
  const top = new THREE.Mesh(new THREE.PlaneGeometry(2 * gw, depth), netMat());
  top.position.set(0, sign * (fy + depth / 2), gh); top.material.map.repeat.set(6, 2); scene.add(top);
}

// A panelled ball texture (meridians/parallels + accent panels) so the ball's
// rotation is actually visible on the sphere.
function ballTexture() {
  const W = 256, H = 128, cv = document.createElement('canvas'); cv.width = W; cv.height = H;
  const x = cv.getContext('2d');
  x.fillStyle = '#d6d6d6'; x.fillRect(0, 0, W, H);
  x.strokeStyle = '#8f8f8f'; x.lineWidth = 2;
  for (let i = 0; i <= 8; i++) { const px = i / 8 * W; x.beginPath(); x.moveTo(px, 0); x.lineTo(px, H); x.stroke(); }
  for (let j = 1; j < 4; j++) { const py = j / 4 * H; x.beginPath(); x.moveTo(0, py); x.lineTo(W, py); x.stroke(); }
  x.fillStyle = '#9aa7c0'; x.fillRect(W * 0.12, H * 0.36, W * 0.1, H * 0.28);
  x.fillStyle = '#c79a9a'; x.fillRect(W * 0.62, H * 0.12, W * 0.08, H * 0.2);
  const t = new THREE.CanvasTexture(cv); t.colorSpace = THREE.SRGBColorSpace; return t;
}
function makeBall() {
  const m = new THREE.Mesh(new THREE.SphereGeometry(F.ball_radius, 32, 20),
    new THREE.MeshStandardMaterial({ map: ballTexture(), roughness: .4, metalness: .1 }));
  m.castShadow = true; scene.add(m); return m;
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

// A boost-amount pill that floats above a car: a rounded track that refills
// left-to-right with the boost level (tinted green/amber/red), the value drawn
// inside. The canvas is redrawn only when the integer value changes (boost moves
// most frames, but the cars are few).
function pillPath(x, X, Y, W, H, r) {
  x.beginPath();
  x.moveTo(X + r, Y);
  x.arcTo(X + W, Y, X + W, Y + H, r);
  x.arcTo(X + W, Y + H, X, Y + H, r);
  x.arcTo(X, Y + H, X, Y, r);
  x.arcTo(X, Y, X + W, Y, r);
  x.closePath();
}
function makeBoostPill() {
  const cv = document.createElement('canvas'); cv.width = 256; cv.height = 72;
  const tex = new THREE.CanvasTexture(cv); tex.colorSpace = THREE.SRGBColorSpace;
  const spr = new THREE.Sprite(new THREE.SpriteMaterial({ map: tex, depthTest: false, transparent: true }));
  spr.scale.set(320, 90, 1); spr.userData = { cv, tex, val: -1 };
  return spr;
}
function setBoostPill(spr, n) {
  const u = spr.userData; if (u.val === n) return; u.val = n;
  const x = u.cv.getContext('2d'); x.clearRect(0, 0, 256, 72);
  const PX = 8, PY = 12, W = 256 - PX * 2, H = 72 - PY * 2, R = H / 2;
  pillPath(x, PX, PY, W, H, R); x.fillStyle = 'rgba(0,0,0,.6)'; x.fill();
  x.save(); pillPath(x, PX, PY, W, H, R); x.clip();
  x.fillStyle = n > 50 ? '#22dd66' : n > 20 ? '#e0b020' : '#dd4030';
  x.fillRect(PX, PY, W * Math.max(0, Math.min(n, 100)) / 100, H); x.restore();
  pillPath(x, PX, PY, W, H, R); x.lineWidth = 3; x.strokeStyle = 'rgba(255,255,255,.28)'; x.stroke();
  x.font = 'bold 38px sans-serif'; x.textAlign = 'center'; x.textBaseline = 'middle';
  x.lineWidth = 6; x.strokeStyle = 'rgba(0,0,0,.85)'; x.strokeText(n, 128, 38);
  x.fillStyle = '#fff'; x.fillText(n, 128, 38);
  u.tex.needsUpdate = true;
}

function buildCars() {
  const map = new Map();
  for (const pl of S.players) {
    const hex = TEAM[pl.team] ?? 0x9aa4b2;
    const g = new THREE.Group();
    // wedge-shaped car: chassis + sloped hood + raised cockpit with a windshield,
    // a rear wing, a white nose (forward / +X), and rimmed wheels.
    const paint = new THREE.MeshStandardMaterial({ color: hex, roughness: .4, metalness: .35 });
    const dark = new THREE.MeshStandardMaterial({ color: new THREE.Color(hex).multiplyScalar(.6), roughness: .5, metalness: .3 });
    const chassis = new THREE.Mesh(new THREE.BoxGeometry(120, 84, 22), paint);
    chassis.position.z = 16; chassis.castShadow = true; g.add(chassis);
    const cabin = new THREE.Mesh(new THREE.BoxGeometry(56, 72, 22), dark);
    cabin.position.set(-10, 0, 33); cabin.castShadow = true; g.add(cabin);
    const wind = new THREE.Mesh(new THREE.BoxGeometry(30, 66, 22),
      new THREE.MeshStandardMaterial({ color: 0x1b2733, roughness: .25, metalness: .5 }));
    wind.position.set(20, 0, 30); wind.rotation.y = -0.6; g.add(wind);
    const hood = new THREE.Mesh(new THREE.BoxGeometry(44, 82, 12), paint);
    hood.position.set(44, 0, 18); hood.castShadow = true; g.add(hood);
    const wing = new THREE.Mesh(new THREE.BoxGeometry(8, 76, 5), dark);
    wing.position.set(-58, 0, 40); g.add(wing);
    for (const wy of [-30, 30]) { const s = new THREE.Mesh(new THREE.BoxGeometry(6, 5, 16), dark); s.position.set(-56, wy, 32); g.add(s); }
    const nose = new THREE.Mesh(new THREE.BoxGeometry(8, 84, 18),
      new THREE.MeshStandardMaterial({ color: 0xffffff, emissive: 0x333333 }));
    nose.position.set(61, 0, 16); g.add(nose);
    const tireMat = new THREE.MeshStandardMaterial({ color: 0x141414, roughness: .9 });
    const rimMat = new THREE.MeshStandardMaterial({ color: 0x9aa0a6, roughness: .4, metalness: .6 });
    for (const [wx, wy] of [[44, 46], [44, -46], [-44, 46], [-44, -46]]) {
      const tire = new THREE.Mesh(new THREE.CylinderGeometry(16, 16, 16, 16), tireMat);
      tire.position.set(wx, wy, 16); tire.castShadow = true; g.add(tire);
      const rim = new THREE.Mesh(new THREE.CylinderGeometry(8, 8, 17, 12), rimMat);
      rim.position.set(wx, wy, 16); g.add(rim);
    }
    const label = makeLabel(pl.name, hex); label.position.set(0, 0, 210); g.add(label);
    const boostPill = makeBoostPill(); boostPill.position.set(0, 0, 345); g.add(boostPill);
    scene.add(g);
    const tg = new THREE.BufferGeometry();
    tg.setAttribute('position', new THREE.BufferAttribute(new Float32Array(64 * 3), 3));
    const trail = new THREE.Line(tg, new THREE.LineBasicMaterial({ color: hex, transparent: true, opacity: .55 }));
    trail.frustumCulled = false; scene.add(trail);
    const callout = new THREE.Sprite(new THREE.SpriteMaterial({ transparent: true, depthTest: false, opacity: 0 }));
    callout.scale.set(780, 174, 1); callout.visible = false; scene.add(callout);
    const ring = new THREE.Mesh(new THREE.RingGeometry(108, 150, 36),
      new THREE.MeshBasicMaterial({ color: 0xffd166, transparent: true, opacity: .85, side: THREE.DoubleSide, depthWrite: false }));
    ring.position.z = 3; ring.visible = false; scene.add(ring);
    const flame = new THREE.Mesh(new THREE.ConeGeometry(20, 72, 12),
      new THREE.MeshBasicMaterial({ color: 0xff9a3c, transparent: true, opacity: .9, blending: THREE.AdditiveBlending, depthWrite: false }));
    flame.rotation.z = Math.PI / 2; flame.position.set(-78, 0, 18); flame.visible = false; g.add(flame);
    map.set(pl.pri, { g, label, boostPill, trail, callout, ring, flame });
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
    out.set(ca.pri, { p: c2 ? lerp3(ca.p, c2.p, w) : ca.p, q, boost: ca.boost, role: ca.role });
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
function writeTrail(line, pts) {
  const arr = line.geometry.attributes.position.array, n = Math.min(pts.length, 64);
  for (let i = 0; i < n; i++) { arr[i * 3] = pts[i][0]; arr[i * 3 + 1] = pts[i][1]; arr[i * 3 + 2] = pts[i][2] + 4; }
  line.geometry.setDrawRange(0, n); line.geometry.attributes.position.needsUpdate = true;
}
function ballTrailPoints(t) {
  const i1 = frameIndex(t), t0 = t - 1.0; let i0 = i1;
  while (i0 > 0 && frames[i0].t > t0) i0--;
  const pts = [];
  for (let i = i0; i <= i1; i++) if (frames[i].ball) pts.push(frames[i].ball);
  return pts;
}

// Skill callouts: a fading "★ <skill>" label above the car that just performed
// it (within ~1.4s), so detected skills surface in the 3D scene, not just the
// ticker. Textures are cached per skill name.
const skillEvents = S.events.filter(e => e.kind === 'skill');
// Per-touch ΔV (scoring-prob swing) keyed by "pri:time" — a ball-contact skill's
// (pri, time) matches its touch exactly, so its callout can pick up the swing.
const touchDv = new Map();
for (const e of S.events) if (e.kind === 'touch' && e.dv != null) touchDv.set(e.pri + ':' + e.t.toFixed(2), e.dv);
const calloutTexCache = new Map();
function calloutTex(label, dv) {
  const key = label + '|' + (dv == null ? '' : dv.toFixed(3));
  if (calloutTexCache.has(key)) return calloutTexCache.get(key);
  const cv = document.createElement('canvas'); cv.width = 340; cv.height = 76;
  const x = cv.getContext('2d');
  // Tint the callout by the swing the touch caused: green helped, red hurt.
  const pos = dv != null && dv > 0.0005, neg = dv != null && dv < -0.0005;
  const edge = pos ? '#3fb950' : neg ? '#f85149' : '#56d4dd';
  const ink = pos ? '#7ee2a8' : neg ? '#ff9b95' : '#8af0f7';
  x.fillStyle = 'rgba(8,32,36,.8)'; x.fillRect(0, 0, 340, 76);
  x.strokeStyle = edge; x.lineWidth = 4; x.strokeRect(2, 2, 336, 72);
  x.font = 'bold 28px sans-serif'; x.fillStyle = ink;
  x.textAlign = 'center'; x.textBaseline = 'middle';
  const tail = dv != null ? '  ' + (dv >= 0 ? '+' : '') + dv.toFixed(2) : '';
  x.fillText('★ ' + label.slice(0, 16) + tail, 170, 40);
  const t = new THREE.CanvasTexture(cv); calloutTexCache.set(key, t); return t;
}
function updateCallouts(t, st) {
  for (const [pri, o] of cars) {
    let best = null;
    for (const e of skillEvents) { if (e.t > t) break; if (e.pri === pri && t - e.t <= 1.4) best = e; }
    const c = st.cars.get(pri);
    if (best && c && o.callout) {
      const age = t - best.t, name = best.label.split(' — ')[0];
      o.callout.material.map = calloutTex(name, touchDv.get(best.pri + ':' + best.t.toFixed(2)));
      o.callout.material.opacity = Math.max(0, 1 - age / 1.4);
      o.callout.position.set(c.p[0], c.p[1], c.p[2] + 510);
      o.callout.visible = true;
    } else if (o.callout) { o.callout.visible = false; }
  }
}

let show = { trails: true, labels: true, boost: true, pads: true };
let prevBall = null;
const prevBoost = new Map();
function applyState(st) {
  if (st.ball) {
    ball.visible = true; ball.position.set(...st.ball);
    if (prevBall) { // roll the ball by how far it moved
      const dx = st.ball[0] - prevBall[0], dy = st.ball[1] - prevBall[1], d = Math.hypot(dx, dy);
      if (d > 1) ball.rotateOnWorldAxis(new THREE.Vector3(-dy, dx, 0).normalize(), d / F.ball_radius);
    }
    prevBall = st.ball;
    ballTrail.visible = show.trails;
    if (show.trails) writeTrail(ballTrail, ballTrailPoints(T));
  } else { ball.visible = false; ballTrail.visible = false; prevBall = null; }
  for (const [pri, o] of cars) {
    const c = st.cars.get(pri);
    const live = !!c;
    o.g.visible = live; o.label.visible = live && show.labels;
    o.trail.visible = live && show.trails;
    o.boostPill.visible = live && show.boost;
    if (!live) { o.flame.visible = false; continue; }
    o.g.position.set(...c.p); o.g.quaternion.copy(c.q);
    o.ring.position.set(c.p[0], c.p[1], 3); o.ring.visible = (c.role === 1);
    if (show.boost) setBoostPill(o.boostPill, c.boost);
    // boost flame: visible while boost is dropping (i.e. boosting)
    o.flame.visible = c.boost < (prevBoost.get(pri) ?? c.boost) - 0.5;
    if (o.flame.visible) { const s = 0.7 + Math.random() * 0.5; o.flame.scale.set(s, 0.9 + Math.random() * 0.5, s); }
    prevBoost.set(pri, c.boost);
    if (show.trails) writeTrail(o.trail, trailPoints(pri, T));
  }
}

// --- HUD ---
const esc = s => s.replace(/[&<>]/g, m => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;' }[m]));
const fmt = s => { const m = Math.floor(s / 60); return m + ':' + (s % 60).toFixed(1).padStart(4, '0'); };
const dur = S.duration_s;
// Live scoreboard: tally goals up to the playhead so the score counts up during
// playback, rather than always showing the final result. (Goal events carry the
// scoring team; S.events is time-sorted, so the early break is safe.)
const goalEvents = S.events.filter(e => e.kind === 'goal');
const scoreEl = document.getElementById('scoreboard');
let lastScoreKey = '';
function updateScore(t) {
  let b = 0, o = 0;
  for (const e of goalEvents) { if (e.t > t + 1e-3) break; if (e.team === 0) b++; else if (e.team === 1) o++; }
  const key = b + '-' + o;
  if (key === lastScoreKey) return;
  lastScoreKey = key;
  scoreEl.innerHTML =
    `<span style="color:#3b82f6">BLUE ${b}</span> — ` +
    `<span style="color:#f97316">${o} ORANGE</span>`;
}
updateScore(0);
document.getElementById('mapline').textContent = (S.map || 'replay') + '  ·  ' + S.replay_id;
if (S.non_standard_map) document.getElementById('warn').style.display = '';
const plistEl = document.getElementById('players');
const tickerEl = document.getElementById('ticker');
const possEl = document.getElementById('poss');

// ΔV (scoring-prob swing) formatting, shared by the roster impact chip + ticker.
function dvColor(v) { return v > 0.0005 ? '#3fb950' : v < -0.0005 ? '#f85149' : '#8b949e'; }
function fmtDv(v) { return (v >= 0 ? '+' : '') + v.toFixed(3); }
// A player's total-impact chip (sum ΔV of their touches), or '' if not attached.
function impChip(v) {
  if (v == null) return '';
  return `<span class="imp" style="color:${dvColor(v)}" title="impact: total scoring-prob swing (ΔV) from this player's touches — + helped, − hurt">Δ${fmtDv(v)}</span>`;
}

// Player rows are built once; only the boost value + follow highlight change.
const rowEls = new Map();
for (const pl of S.players) {
  const row = document.createElement('div');
  row.className = 'row'; row.dataset.pri = pl.pri;
  const css = TEAM_CSS[pl.team] ?? '#9aa4b2';
  row.innerHTML =
    `<div class="r1"><span class="dot" style="background:${css}"></span>${esc(pl.name)}<i class="role"></i>${impChip(pl.impact)}</div>` +
    `<div class="r2"><span class="ga">G${pl.goals} A${pl.assists} Sv${pl.saves}</span><span class="sp"></span><span class="bz"><i class="f"></i><b>0</b></span></div>`;
  row.onclick = () => setCam('player', pl.pri);
  plistEl.appendChild(row);
  rowEls.set(pl.pri, row);
}
function carSpeed(pri, t) {
  const i = frameIndex(t); if (i + 1 >= frames.length) return 0;
  const a = frames[i].cars.find(c => c.pri === pri), b = frames[i + 1].cars.find(c => c.pri === pri);
  if (!a || !b) return 0;
  // cap at ~max car speed so a respawn/demo teleport (position jump) doesn't spike it
  return Math.min(Math.hypot(b.p[0] - a.p[0], b.p[1] - a.p[1], b.p[2] - a.p[2]) * S.hz, 2300);
}
function updatePlayers(st) {
  for (const pl of S.players) {
    const row = rowEls.get(pl.pri), c = st ? st.cars.get(pl.pri) : null;
    const bv = c ? c.boost : 0, bf = row.querySelector('.bz .f');
    row.querySelector('.bz b').textContent = bv;
    bf.style.width = bv + '%';
    bf.style.background = bv > 50 ? '#22dd66' : bv > 20 ? '#e0b020' : '#dd4030';
    row.querySelector('.sp').textContent = c ? Math.round(carSpeed(pl.pri, T) * 0.036) + ' kph' : '';
    const r = row.querySelector('.role');
    r.textContent = c && c.role ? (c.role === 1 ? '1ST' : '2ND') : '';
    r.className = 'role' + (c && c.role === 1 ? ' first' : '');
    row.classList.toggle('follow', cam.mode === 'player' && cam.pri === pl.pri);
  }
}
function possAt(t) { let team = null; for (const e of S.events) { if (e.t > t + 1e-3) break; if (e.kind === 'touch') team = e.team; } return team; }

// The ticker is rebuilt only when the most-recent-event index changes.
let lastTickerIdx = -2;
function updateHud(t, st) {
  document.getElementById('clock').textContent = fmt(t) + ' / ' + fmt(dur);
  updateScore(t);
  updatePlayers(st);
  const pt = possAt(t);
  possEl.innerHTML = pt == null ? '' : ` · poss <span style="color:${TEAM_CSS[pt] ?? '#888'}">${pt === 0 ? 'BLUE' : 'ORANGE'}</span>`;
  let i = S.events.length - 1;
  while (i >= 0 && S.events[i].t > t + 1e-3) i--;
  if (i !== lastTickerIdx) {
    lastTickerIdx = i;
    const recent = S.events.slice(Math.max(0, i - 6), i + 1);
    tickerEl.innerHTML = recent.map(e =>
      `<div class="ev ${e.kind}" data-t="${e.t}"><span>${fmt(e.t)}</span>` +
      (e.dv != null ? `<span class="d" style="color:${dvColor(e.dv)}">${fmtDv(e.dv)}</span>` : '') +
      `${esc(e.label)}</div>`).join('');
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

// momentum strip: P(team 0 scores the next goal) across the match (filled from
// the 0.5 baseline, blue above / orange below), with a cursor at the current time
const wpCanvas = document.getElementById('wp'), wpCursor = document.getElementById('wpcursor');
function drawWP() {
  const wp = S.win_prob;
  if (!wp || !wp.length) { wpCanvas.style.display = 'none'; wpCursor.style.display = 'none'; return; }
  const W = wpCanvas.clientWidth || 600, H = 20;
  wpCanvas.width = W; wpCanvas.height = H;
  const x = wpCanvas.getContext('2d'); x.clearRect(0, 0, W, H);
  const n = wp.length, mid = H / 2;
  for (let i = 0; i < n; i++) {
    const px = i / (n - 1) * W, p = wp[i], py = (1 - p) * H;
    x.strokeStyle = p >= 0.5 ? '#3b82f6' : '#f97316';
    x.globalAlpha = Math.min(1, Math.abs(p - 0.5) * 2 + 0.12);
    x.beginPath(); x.moveTo(px, mid); x.lineTo(px, py); x.stroke();
  }
  x.globalAlpha = 1;
  x.strokeStyle = '#30363d'; x.beginPath(); x.moveTo(0, mid); x.lineTo(W, mid); x.stroke();
  x.strokeStyle = '#cdd9e5'; x.lineWidth = 1; x.beginPath();
  for (let i = 0; i < n; i++) { const px = i / (n - 1) * W, py = (1 - wp[i]) * H; i ? x.lineTo(px, py) : x.moveTo(px, py); }
  x.stroke();
}
drawWP();

// pressure strip: which half the ball sits in over the match (ball y in team 0's
// attack frame), blue pressing above the midline / orange below — ballchasing's
// signature "pressure" view. Drawn once from the playback frames.
const presCanvas = document.getElementById('pressure');
function drawPressure() {
  const fr = S.frames;
  if (!fr.length) { presCanvas.style.display = 'none'; return; }
  const W = presCanvas.clientWidth || 600, H = 14;
  presCanvas.width = W; presCanvas.height = H;
  const x = presCanvas.getContext('2d'); x.clearRect(0, 0, W, H);
  const sign0 = (S.attack_sign && S.attack_sign['0']) || 1;
  const Y = S.field.back_wall_y || 5120, mid = H / 2;
  const stride = Math.max(1, Math.floor(fr.length / (W * 2)));
  for (let i = 0; i < fr.length; i += stride) {
    const f = fr[i]; if (!f.ball) continue;
    const p = Math.max(-1, Math.min(1, f.ball[1] * sign0 / Y)); // +1 ⇒ blue pressing
    const px = (f.t / dur) * W, py = mid - p * mid;
    x.strokeStyle = p >= 0 ? '#3b82f6' : '#f97316';
    x.globalAlpha = Math.min(1, Math.abs(p) + 0.1);
    x.beginPath(); x.moveTo(px, mid); x.lineTo(px, py); x.stroke();
  }
  x.globalAlpha = 1;
  x.strokeStyle = '#30363d'; x.beginPath(); x.moveTo(0, mid); x.lineTo(W, mid); x.stroke();
}
drawPressure();

// --- playback ---
let T = 0, playing = true, speed = 1, loop = false, last = performance.now(), lastState = null;
let loopA = null, loopB = null; // A–B film-review loop
function updateLoopUI() {
  const r = document.getElementById('loopRegion'), on = loopA != null && loopB != null && loopB > loopA;
  r.style.display = on ? 'block' : 'none';
  if (on) { r.style.left = (loopA / dur * 100) + '%'; r.style.width = ((loopB - loopA) / dur * 100) + '%'; }
}
const tl = document.getElementById('timeline'), playBtn = document.getElementById('play');
tl.max = dur; tl.step = 0.01; playBtn.innerHTML = ICON_PAUSE;
function setPlaying(p) { playing = p; playBtn.innerHTML = p ? ICON_PAUSE : ICON_PLAY; }
function seek(t) { T = Math.min(Math.max(t, 0), dur); tl.value = T; }
playBtn.onclick = () => setPlaying(!playing);
tl.oninput = () => { T = parseFloat(tl.value); setPlaying(false); };
document.getElementById('speed').onchange = e => { speed = parseFloat(e.target.value); };
document.getElementById('loop').onchange = e => { loop = e.target.checked; };
for (const id of ['Trails', 'Labels', 'Boost', 'Pads']) {
  document.getElementById('tg' + id).onchange = e => { show[id.toLowerCase()] = e.target.checked; };
}
document.getElementById('tgHeat').onchange = e => {
  heatMesh.visible = e.target.checked;
  if (e.target.checked) buildHeat(heatSubject);
};
function jump(kind, dir) {
  const ts = S.events.filter(e => e.kind === kind).map(e => e.t);
  if (dir > 0) { const n = ts.find(x => x > T + 0.05); if (n != null) { seek(n); setPlaying(false); } }
  else { const p = [...ts].reverse().find(x => x < T - 0.05); if (p != null) { seek(p); setPlaying(false); } }
}
addEventListener('keydown', e => {
  if (e.target.tagName === 'INPUT' || e.target.tagName === 'SELECT') return;
  if (e.code === 'Space') { e.preventDefault(); setPlaying(!playing); }
  else if (e.code === 'ArrowRight') seek(T + 1);
  else if (e.code === 'ArrowLeft') seek(T - 1);
  else if (e.key === 'n') jump('goal', 1);
  else if (e.key === 'p') jump('goal', -1);
  else if (e.key === 'k') jump('kickoff', 1);
  else if (e.key === 'j') jump('kickoff', -1);
  else if (e.key === '0') setCam('free');
  else if (e.key === 'd') setDrawMode(!drawMode);
  else if (e.key === 'z' && drawMode) undoLast();
  else if (e.key === 's') exportPNG();
  else if (e.key === '?') helpEl.style.display = helpEl.style.display === 'flex' ? 'none' : 'flex';
  else if (e.key === 'i') { loopA = T; updateLoopUI(); }
  else if (e.key === 'o') { loopB = T; updateLoopUI(); }
  else if (e.key === 'x') { loopA = loopB = null; updateLoopUI(); }
});

function animate(now) {
  requestAnimationFrame(animate);
  const dt = (now - last) / 1000; last = now;
  const seg = loopA != null && loopB != null && loopB > loopA;
  const segEnd = seg ? loopB : dur, segStart = seg ? loopA : 0;
  if (playing) { T += dt * speed; if (T >= segEnd) { if (loop || seg) T = segStart; else { T = dur; setPlaying(false); } } }
  tl.value = T;
  wpCursor.style.left = (T / dur * 100) + '%';
  const st = stateAt(T); lastState = st;
  applyState(st);
  updatePads(T);
  updateHud(T, st);
  updateCallouts(T, st);
  drawMinimap(st);
  for (const e of demoEvents) if (e.t > prevT && e.t <= T) { const c = st.cars.get(e.pri); if (c) spawnFlash(c.p); }
  prevT = T;
  updateFlashes(now);
  // eased preset transition
  if (camTween) {
    const e = Math.min(1, (now - camTween.start) / camTween.dur);
    const k = e < .5 ? 2 * e * e : 1 - Math.pow(-2 * e + 2, 2) / 2; // easeInOutQuad
    camera.position.lerpVectors(camTween.fromPos, camTween.toPos, k);
    controls.target.lerpVectors(camTween.fromTgt, camTween.toTgt, k);
    if (e >= 1) camTween = null;
  }
  // follow cameras
  if (cam.mode === 'broadcast') {
    // fixed elevated sideline position that pans to track the ball
    if (st.ball) controls.target.lerp(new THREE.Vector3(...st.ball), 0.08);
    camera.position.lerp(BROADCAST_POS, 0.05);
  } else {
    let focus = null;
    if (cam.mode === 'ball' && st.ball) focus = new THREE.Vector3(...st.ball);
    else if (cam.mode === 'player' && cam.pri != null) { const c = st.cars.get(cam.pri); if (c) focus = new THREE.Vector3(...c.p); }
    if (focus) {
      controls.target.lerp(focus, 0.18);
      const desired = focus.clone().add(new THREE.Vector3(0, -1500, 750));
      camera.position.lerp(desired, 0.06);
    }
  }
  controls.update();
  renderer.render(scene, camera);
  window.__rendered = (window.__rendered || 0) + 1; // signal for the headless smoke test
  if (window.__rendered === 1) document.getElementById('loading').style.display = 'none';
}
requestAnimationFrame(animate);

addEventListener('resize', () => {
  camera.aspect = innerWidth / innerHeight;
  camera.updateProjectionMatrix();
  renderer.setSize(innerWidth, innerHeight);
  drawWP();
  drawPressure();
  sizeDraw();
});
</script>
</body>
</html>
"##;
