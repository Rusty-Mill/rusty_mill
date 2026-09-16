/**
 * ReverseTrace.jsx
 *
 * "Why can't I have this dashboard?" -- the reverse-trace view.
 *
 * Leadership picks a desired outcome; the graph re-lays out with the
 * outcome on the RIGHT, the domains it draws on in the middle and the
 * concrete sources (systems, files, people) on the LEFT -- the reverse of
 * the Mesh Ops producer→consumer flow, deliberately. The outcome
 * "requests" (a cyan pulse runs right-to-left), then data particles try
 * to flow back from the sources: they arrive smoothly through satisfied
 * domains, trickle through degraded ones, and die at blocked ones, whose
 * node pulses red.
 *
 * Everything is scenario-local and simulated (spec §3 non-goals). The
 * scenario JSON and the trace algorithm come from `rusty-meshed-trace`
 * (see reverseTrace.js); what-if maturity changes live in component state
 * and reset on reload.
 */

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import BASE_SCENARIO from "./scenarios/acquisition_status.json";
import {
  EDGE_STATE, MATURITY, domainById, domainState, fidelitySummary, maturityColor,
  maturityLabel, sourcesOf, toMarkdown, traceOutcome, withMaturity,
} from "./reverseTrace.js";

// ═══════════════════════════════════════════════════════════════════════════
// LAYOUT
// ═══════════════════════════════════════════════════════════════════════════

const VB_W = 900, VB_H = 520;
const COL = { source: 125, domain: 465, outcome: 800 };
const NODE = {
  source:  { w: 176, h: 40 },
  domain:  { w: 184, h: 58 },
  outcome: { w: 176, h: 78 },
};
const REQUEST_MS = 1300;   // how long the outcome→sources request pulse runs
const ACCENT = "#22d3ee";

const KIND_TAG = { system: "SYS", file: "FILE", person: "PERSON", process: "PROC" };

function cadence(secs) {
  if (!secs) return "on request";
  if (secs < 3600) return `${Math.round(secs / 60)}m refresh`;
  if (secs < 86400) return `${Math.round(secs / 3600)}h refresh`;
  if (secs < 604800) return secs === 86400 ? "daily" : `${Math.round(secs / 86400)}d refresh`;
  if (secs < 2592000) return secs === 604800 ? "weekly" : `${Math.round(secs / 604800)}w refresh`;
  return "monthly";
}

/** Cubic bezier between two points (same curve family as Mesh Ops). */
function curve(a, b) {
  const dx = (b.x - a.x) * 0.42;
  return `C${a.x + dx},${a.y} ${b.x - dx},${b.y} ${b.x},${b.y}`;
}
const pathThrough = (pts) => `M${pts[0].x},${pts[0].y} ` + pts.slice(1).map((p, i) => curve(pts[i], p)).join(" ");

/** Positions for the domains the outcome requires and their sources. */
function layout(scenario, report) {
  const domains = report.edges
    .filter((e) => e.from.kind === "domain")
    .map((e) => domainById(scenario, e.from.id))
    .filter(Boolean);
  const sources = domains.flatMap((d) => sourcesOf(scenario, d.id));
  const top = 58, bottom = VB_H - 34;
  const spread = (n, i) => (n <= 1 ? (top + bottom) / 2 : top + ((bottom - top) * i) / (n - 1));
  return {
    domains,
    sources,
    domainPos: Object.fromEntries(domains.map((d, i) => [d.id, { x: COL.domain, y: spread(domains.length, i) }])),
    sourcePos: Object.fromEntries(sources.map((s, i) => [s.id, { x: COL.source, y: spread(sources.length, i) }])),
    outcomePos: { x: COL.outcome, y: VB_H / 2 + 8 },
  };
}

let _pid = 0;
const rand = (lo, hi) => Math.random() * (hi - lo) + lo;
const pick = (arr) => arr[Math.floor(Math.random() * arr.length)];

// ═══════════════════════════════════════════════════════════════════════════
// ROOT
// ═══════════════════════════════════════════════════════════════════════════

export default function ReverseTrace() {
  const [outcomeId, setOutcomeId] = useState(BASE_SCENARIO.outcomes[0].id);
  const [overrides, setOverrides] = useState({});          // what-if: domainId → level
  const [particles, setParticles] = useState([]);
  const [bursts, setBursts] = useState([]);                // particle deaths at blocked domains
  const [phase, setPhase] = useState("requesting");        // "requesting" | "flowing"
  const [showMarkdown, setShowMarkdown] = useState(false);
  const [copied, setCopied] = useState(false);

  // Effective scenario = shipped scenario + what-if overrides.
  const scenario = useMemo(
    () => Object.entries(overrides).reduce((s, [id, lvl]) => withMaturity(s, id, lvl), BASE_SCENARIO),
    [overrides],
  );
  const report = useMemo(() => traceOutcome(scenario, outcomeId), [scenario, outcomeId]);
  const geo = useMemo(() => layout(scenario, report), [scenario, report]);
  const verdicts = useMemo(
    () => Object.fromEntries(scenario.outcomes.map((o) => [o.id, traceOutcome(scenario, o.id).achievable])),
    [scenario],
  );
  const markdown = useMemo(() => toMarkdown(report, scenario), [report, scenario]);

  const geoRef = useRef(geo), reportRef = useRef(report), phaseRef = useRef(phase);
  useEffect(() => { geoRef.current = geo; }, [geo]);
  useEffect(() => { reportRef.current = report; }, [report]);
  useEffect(() => { phaseRef.current = phase; }, [phase]);

  // ── Request pulse: outcome → domains → sources, right-to-left ───────────
  useEffect(() => {
    setParticles([]);
    setBursts([]);
    setPhase("requesting");
    const { domains, domainPos, sourcePos, outcomePos } = geoRef.current;
    const now = Date.now();
    const pulses = domains.flatMap((d) =>
      sourcesOf(scenario, d.id).map((s) => ({
        id: ++_pid,
        path: pathThrough([outcomePos, domainPos[d.id], sourcePos[s.id]]),
        color: ACCENT, r: 2.4, dur: REQUEST_MS - 250, start: now, request: true,
      })),
    );
    setParticles(pulses);
    const t = setTimeout(() => setPhase("flowing"), REQUEST_MS);
    return () => clearTimeout(t);
    // Re-run only when the outcome changes: a what-if slider tweak should
    // not restart the request animation.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [outcomeId]);

  // ── Data particles: sources → domains → outcome ─────────────────────────
  const spawn = useCallback(() => {
    if (phaseRef.current !== "flowing") return;
    const { sources, domainPos, sourcePos, outcomePos } = geoRef.current;
    const rep = reportRef.current;
    if (sources.length === 0) return;
    const src = pick(sources);
    const state = domainState(rep, src.domain);
    if (!state || state === "missing") return;                     // nothing flows from an optional, unmet domain
    if (state === "degraded" && Math.random() < 0.5) return;        // throttled
    const color = EDGE_STATE[state].color;
    const dies = state === "blocked";
    const pts = dies
      ? [sourcePos[src.id], domainPos[src.domain]]
      : [sourcePos[src.id], domainPos[src.domain], outcomePos];
    const dur = dies ? rand(900, 1300) : state === "degraded" ? rand(2600, 3600) : rand(1500, 2100);
    const id = ++_pid;
    setParticles((p) => [...p.slice(-80), { id, path: pathThrough(pts), color, r: dies ? 3 : 2.6, dur, start: Date.now() }]);
    if (dies) {
      const at = domainPos[src.domain];
      setTimeout(() => setBursts((b) => [...b.slice(-12), { id, x: at.x - NODE.domain.w / 2, y: at.y, start: Date.now() }]), dur);
    }
  }, []);

  useEffect(() => {
    let alive = true;
    const tick = () => {
      if (!alive) return;
      spawn();
      setTimeout(tick, rand(90, 220));
    };
    const t = setTimeout(tick, 100);
    const gc = setInterval(() => {
      const now = Date.now();
      setParticles((p) => p.filter((x) => now - x.start < x.dur + 200));
      setBursts((b) => b.filter((x) => now - x.start < 700));
    }, 500);
    return () => { alive = false; clearTimeout(t); clearInterval(gc); };
  }, [spawn]);

  // ── What-if handlers ────────────────────────────────────────────────────
  const setLevel = (domainId, level) => {
    setOverrides((o) => {
      const base = domainById(BASE_SCENARIO, domainId).maturity;
      const next = { ...o };
      if (level === base) delete next[domainId]; else next[domainId] = level;
      return next;
    });
  };
  const resetAll = () => setOverrides({});

  const copyMarkdown = async () => {
    try {
      await navigator.clipboard.writeText(markdown);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch {
      setShowMarkdown(true);
    }
  };

  const fidelity = fidelitySummary(report.achievable);
  const counts = { blocked: 0, degraded: 0, missing: 0, satisfied: 0 };
  report.edges.filter((e) => e.from.kind === "domain").forEach((e) => { counts[e.state]++; });

  return (
    <div style={S.root}>
      <style>{CSS}</style>
      <Header
        scenario={scenario} outcomeId={outcomeId} onSelect={setOutcomeId} verdicts={verdicts}
        fidelity={fidelity} counts={counts} overrideCount={Object.keys(overrides).length}
      />
      <Legend />
      <div style={S.body}>
        <GraphPanel
          scenario={scenario} report={report} geo={geo} particles={particles} bursts={bursts}
          phase={phase} fidelity={fidelity} overrides={overrides}
        />
        <SidePanel
          scenario={scenario} report={report} fidelity={fidelity}
          onCopy={copyMarkdown} copied={copied}
          showMarkdown={showMarkdown} onToggleMarkdown={() => setShowMarkdown((v) => !v)} markdown={markdown}
        />
      </div>
      <WhatIfBar domains={geo.domains} report={report} overrides={overrides} onChange={setLevel} onReset={resetAll} />
    </div>
  );
}

// ═══════════════════════════════════════════════════════════════════════════
// HEADER · OUTCOME SELECTOR · KPIs
// ═══════════════════════════════════════════════════════════════════════════

function Header({ scenario, outcomeId, onSelect, verdicts, fidelity, counts, overrideCount }) {
  return (
    <div style={S.header}>
      <div style={{ display: "flex", alignItems: "baseline", gap: 8, flexShrink: 0 }}>
        <span style={S.wordmarkMain}>MESHED</span>
        <span style={S.wordmarkSub}>REVERSE TRACE</span>
      </div>
      <div style={{ display: "flex", alignItems: "center", gap: 5, flexShrink: 0 }}>
        <div className="live" style={{ width: 7, height: 7, borderRadius: "50%", background: "#fbbf24", boxShadow: "0 0 6px #fbbf24" }} />
        <span style={{ fontSize: 9, color: "#fbbf24", letterSpacing: ".12em" }}>SIM · SCENARIO-LOCAL</span>
      </div>
      <div style={S.divider} />

      {/* Outcome selector */}
      <div style={{ display: "flex", alignItems: "center", gap: 4, overflow: "hidden", minWidth: 0, flexShrink: 1 }}>
        <span style={{ fontSize: 7, color: "#1e3040", letterSpacing: ".16em", marginRight: 4, flexShrink: 0 }}>OUTCOME</span>
        {scenario.outcomes.map((o) => {
          const v = fidelitySummary(verdicts[o.id]);
          const active = o.id === outcomeId;
          return (
            <button key={o.id} onClick={() => onSelect(o.id)} title={o.description}
              style={{ ...S.outcomeBtn, ...(active ? S.outcomeBtnActive : {}) }}>
              <span style={{ width: 6, height: 6, borderRadius: "50%", background: v.color, boxShadow: `0 0 4px ${v.color}`, flexShrink: 0 }} />
              {o.name}
            </button>
          );
        })}
      </div>

      <div style={{ flex: 1 }} />

      {[
        { label: "FIDELITY", val: fidelity.label, color: fidelity.color, wide: true },
        { label: "BLOCKED", val: counts.blocked, color: counts.blocked ? EDGE_STATE.blocked.color : "#334155" },
        { label: "DEGRADED", val: counts.degraded, color: counts.degraded ? EDGE_STATE.degraded.color : "#334155" },
        { label: "MISSING", val: counts.missing, color: counts.missing ? "#94a3b8" : "#334155" },
        { label: "WHAT-IF", val: overrideCount, color: overrideCount ? ACCENT : "#334155" },
      ].map((s) => (
        <div key={s.label} style={{ textAlign: "right", minWidth: s.wide ? 110 : 50, flexShrink: 0 }}>
          <div style={{ fontSize: 7, color: "#1e3040", letterSpacing: ".14em", marginBottom: 2 }}>{s.label}</div>
          <div style={{ fontSize: s.wide ? 12 : 14, fontWeight: 600, color: s.color, whiteSpace: "nowrap" }}>{s.val}</div>
        </div>
      ))}
    </div>
  );
}

// ═══════════════════════════════════════════════════════════════════════════
// LEGEND (always visible -- nobody should have to guess what red means)
// ═══════════════════════════════════════════════════════════════════════════

function Legend() {
  return (
    <div style={S.legend}>
      <span style={S.legendTitle}>MATURITY</span>
      {MATURITY.map((m) => (
        <span key={m.level} title={m.blurb} style={{ display: "flex", alignItems: "center", gap: 4 }}>
          <span style={{ width: 8, height: 8, borderRadius: 2, background: m.color, boxShadow: `0 0 4px ${m.color}` }} />
          <span style={{ fontSize: 8, color: m.color, letterSpacing: ".08em" }}>L{m.level} {m.name.toUpperCase()}</span>
        </span>
      ))}
      <div style={S.divider} />
      <span style={S.legendTitle}>EDGE</span>
      {Object.entries(EDGE_STATE).map(([k, e]) => (
        <span key={k} title={e.blurb} style={{ display: "flex", alignItems: "center", gap: 5 }}>
          <svg width="26" height="8"><line x1="1" y1="4" x2="25" y2="4" stroke={e.color} strokeWidth="1.6" strokeDasharray={e.dash ?? undefined} strokeLinecap="round" /></svg>
          <span style={{ fontSize: 8, color: e.color, letterSpacing: ".08em" }}>{e.label.toUpperCase()}</span>
        </span>
      ))}
      <div style={{ flex: 1 }} />
      <span style={{ fontSize: 8, color: "#1e3040", letterSpacing: ".08em" }}>
        FLOW ← SOURCES · DOMAINS · OUTCOME → · fidelity is capped by the weakest required domain
      </span>
    </div>
  );
}

// ═══════════════════════════════════════════════════════════════════════════
// GRAPH
// ═══════════════════════════════════════════════════════════════════════════

const COLUMNS = [
  { label: "SOURCES · where the data lives today", x: COL.source },
  { label: "DOMAINS · maturity vs. floor", x: COL.domain },
  { label: "OUTCOME · what leadership asked for", x: COL.outcome },
];

function GraphPanel({ scenario, report, geo, particles, bursts, phase, fidelity, overrides }) {
  const { domains, sources, domainPos, sourcePos, outcomePos } = geo;
  const outcome = scenario.outcomes.find((o) => o.id === report.outcome);
  const reqByDomain = Object.fromEntries(outcome.requires.map((r) => [r.domain, r]));

  return (
    <div style={{ flex: 1, position: "relative", overflow: "hidden" }}>
      <svg style={{ position: "absolute", inset: 0, width: "100%", height: "100%", opacity: 0.07 }}>
        <defs>
          <pattern id="rt-grid" width="36" height="36" patternUnits="userSpaceOnUse">
            <path d="M36,0 L0,0 0,36" fill="none" stroke={ACCENT} strokeWidth=".5" />
          </pattern>
        </defs>
        <rect width="100%" height="100%" fill="url(#rt-grid)" />
      </svg>

      <svg viewBox={`0 0 ${VB_W} ${VB_H}`} style={{ width: "100%", height: "100%" }} preserveAspectRatio="xMidYMid meet">
        {COLUMNS.map((c) => (
          <text key={c.label} x={c.x} y={26} textAnchor="middle" fontSize={8} letterSpacing=".16em" fontWeight="600"
            fill="#22d3ee" opacity=".35" fontFamily="'JetBrains Mono',monospace">{c.label}</text>
        ))}

        {/* Edges: source → domain, domain → outcome */}
        {report.edges.map((e, i) => {
          const a = e.from.kind === "source" ? sourcePos[e.from.id] : domainPos[e.from.id];
          const b = e.to.kind === "domain" ? domainPos[e.to.id] : outcomePos;
          if (!a || !b) return null;
          const st = EDGE_STATE[e.state];
          const halfA = e.from.kind === "source" ? NODE.source.w / 2 : NODE.domain.w / 2;
          const halfB = e.to.kind === "domain" ? NODE.domain.w / 2 : NODE.outcome.w / 2;
          return (
            <path key={i} d={pathThrough([{ x: a.x + halfA, y: a.y }, { x: b.x - halfB, y: b.y }])} fill="none"
              stroke={st.color} strokeWidth={e.state === "satisfied" ? 1.4 : 1.1}
              strokeOpacity={e.state === "missing" ? 0.35 : 0.45} strokeDasharray={st.dash ?? undefined} />
          );
        })}

        {/* Particles (request pulse + data flow) */}
        {particles.map((p) => (
          <circle key={p.id} r={p.r} fill={p.color} opacity={p.request ? 0.9 : 1}
            style={{ filter: `drop-shadow(0 0 ${p.request ? 4 : 5}px ${p.color})` }}>
            <animateMotion dur={`${p.dur}ms`} begin="0s" repeatCount="1" fill="freeze" path={p.path} />
          </circle>
        ))}
        {bursts.map((b) => (
          <circle key={`b${b.id}`} className="rt-burst" cx={b.x} cy={b.y} r={4} fill="none" stroke={EDGE_STATE.blocked.color} strokeWidth="1.2" />
        ))}

        {/* Source nodes */}
        {sources.map((s) => {
          const pos = sourcePos[s.id];
          const state = domainState(report, s.domain) ?? "missing";
          const color = EDGE_STATE[state].color;
          const { w, h } = NODE.source;
          return (
            <g key={s.id} transform={`translate(${pos.x},${pos.y})`}>
              <rect x={-w / 2} y={-h / 2} width={w} height={h} rx={3} fill="#040a12" stroke={color} strokeWidth={0.6} strokeOpacity={0.5} />
              <rect x={-w / 2} y={-h / 2} width={3} height={h} rx={1.5} fill={color} opacity={0.7} />
              <text x={-w / 2 + 10} y={-4} fontSize={8.5} fontWeight="600" fill="#7a8fa8" fontFamily="'JetBrains Mono',monospace">
                {truncate(s.name, 26)}
              </text>
              <text x={-w / 2 + 10} y={9} fontSize={6.5} fill="#2d4460" fontFamily="'JetBrains Mono',monospace">
                {KIND_TAG[s.kind]} · struct {s.structure} · avail {s.availability} · {cadence(s.latency_secs)}
              </text>
            </g>
          );
        })}

        {/* Domain nodes */}
        {domains.map((d) => {
          const pos = domainPos[d.id];
          const state = domainState(report, d.id);
          const req = reqByDomain[d.id];
          const color = EDGE_STATE[state].color;
          const mcolor = maturityColor(d.maturity);
          const { w, h } = NODE.domain;
          const changed = overrides[d.id] != null;
          return (
            <g key={d.id} transform={`translate(${pos.x},${pos.y})`} className={state === "blocked" && phase === "flowing" ? "rt-pulse" : ""}>
              {state === "blocked" && (
                <rect x={-w / 2 - 6} y={-h / 2 - 6} width={w + 12} height={h + 12} rx={8} fill={color} opacity=".09" style={{ filter: "blur(10px)" }} />
              )}
              <rect x={-w / 2} y={-h / 2} width={w} height={h} rx={3} fill="#040a12" stroke={color}
                strokeWidth={state === "satisfied" ? 0.8 : 1.2} strokeOpacity={state === "missing" ? 0.4 : 0.8} />
              <rect x={-w / 2} y={-h / 2} width={4} height={h} rx={2} fill={mcolor} />
              <text x={-w / 2 + 12} y={-12} fontSize={9.5} fontWeight="600" fill={state === "missing" ? "#475569" : "#c7d2e0"} fontFamily="'JetBrains Mono',monospace">
                {truncate(d.name, 24)}{changed ? " ·" : ""}
              </text>
              <text x={-w / 2 + 12} y={0} fontSize={6.5} fill="#2d4460" fontFamily="'JetBrains Mono',monospace">{truncate(d.owner, 30)}</text>
              {/* maturity pips: filled up to current, hollow to required */}
              {MATURITY.map((m) => {
                const x = -w / 2 + 14 + m.level * 12;
                const filled = m.level <= d.maturity;
                const needed = m.level <= req.min_maturity;
                return (
                  <rect key={m.level} x={x} y={10} width={9} height={6} rx={1}
                    fill={filled ? maturityColor(d.maturity) : "transparent"}
                    stroke={needed ? (filled ? maturityColor(d.maturity) : color) : "#1a2d42"} strokeWidth={0.8}
                    strokeDasharray={!filled && needed ? "1.5 1.5" : undefined} />
                );
              })}
              <text x={w / 2 - 8} y={16} textAnchor="end" fontSize={7} fill={state === "satisfied" ? EDGE_STATE.satisfied.color : color} fontFamily="'JetBrains Mono',monospace">
                L{d.maturity} · floor L{req.min_maturity} · {req.criticality.toUpperCase()}
              </text>
            </g>
          );
        })}

        {/* Outcome node */}
        {(() => {
          const { w, h } = NODE.outcome;
          const pos = outcomePos;
          return (
            <g transform={`translate(${pos.x},${pos.y})`}>
              <rect x={-w / 2 - 8} y={-h / 2 - 8} width={w + 16} height={h + 16} rx={10} fill={fidelity.color} opacity=".08" style={{ filter: "blur(12px)" }} />
              <rect x={-w / 2} y={-h / 2} width={w} height={h} rx={4} fill="#060f1a" stroke={ACCENT} strokeWidth={1.2} strokeOpacity={0.9} />
              <rect x={-w / 2} y={-h / 2} width={4} height={h} rx={2} fill={ACCENT} />
              <text x={-w / 2 + 12} y={-h / 2 + 16} fontSize={7} letterSpacing=".14em" fill="#1e3a5f" fontFamily="'JetBrains Mono',monospace">
                {phase === "requesting" ? "REQUESTING…" : "OUTCOME"}
              </text>
              <text x={-w / 2 + 12} y={-h / 2 + 32} fontSize={9.5} fontWeight="600" fill={ACCENT} fontFamily="'JetBrains Mono',monospace">
                {truncate(outcome.name, 24)}
              </text>
              <text x={-w / 2 + 12} y={-h / 2 + 49} fontSize={8} fontWeight="600" fill={fidelity.color} fontFamily="'JetBrains Mono',monospace">
                {fidelity.label}
              </text>
              <rect x={-w / 2 + 12} y={-h / 2 + 58} width={w - 24} height={4} rx={2} fill="#0d1e30" />
              <rect x={-w / 2 + 12} y={-h / 2 + 58} width={(w - 24) * fidelity.fraction} height={4} rx={2} fill={fidelity.color} style={{ transition: "width .6s ease" }} />
            </g>
          );
        })()}
      </svg>
    </div>
  );
}

const truncate = (s, n) => (s.length > n ? s.slice(0, n - 1) + "…" : s);

// ═══════════════════════════════════════════════════════════════════════════
// SIDE PANEL · FIDELITY GAUGE · BOTTLENECKS · EXPORT
// ═══════════════════════════════════════════════════════════════════════════

function Gauge({ fidelity }) {
  const W = 200, H = 110, cx = W / 2, cy = 100, r = 84;
  const arc = (from, to) => {
    const a0 = Math.PI * (1 - from), a1 = Math.PI * (1 - to);
    const x0 = cx + r * Math.cos(a0), y0 = cy - r * Math.sin(a0);
    const x1 = cx + r * Math.cos(a1), y1 = cy - r * Math.sin(a1);
    return `M${x0},${y0} A${r},${r} 0 ${to - from > 0.5 ? 1 : 0} 1 ${x1},${y1}`;
  };
  const f = Math.max(0.001, fidelity.fraction);
  return (
    <svg width={W} height={H} viewBox={`0 0 ${W} ${H}`} style={{ display: "block", margin: "0 auto" }}>
      <path d={arc(0, 1)} fill="none" stroke="#0d1e30" strokeWidth="10" strokeLinecap="round" />
      <path d={arc(0, f)} fill="none" stroke={fidelity.color} strokeWidth="10" strokeLinecap="round"
        style={{ filter: `drop-shadow(0 0 6px ${fidelity.color})`, transition: "d .6s ease" }} />
      <text x={cx} y={cy - 22} textAnchor="middle" fontSize="20" fontWeight="600" fill={fidelity.color} fontFamily="'JetBrains Mono',monospace">
        {Math.round(fidelity.fraction * 100)}%
      </text>
      <text x={cx} y={cy - 4} textAnchor="middle" fontSize="8" letterSpacing=".14em" fill={fidelity.color} fontFamily="'JetBrains Mono',monospace">
        {fidelity.label}
      </text>
    </svg>
  );
}

function Pips({ current, required }) {
  return (
    <span style={{ display: "inline-flex", gap: 2, verticalAlign: "middle" }}>
      {MATURITY.map((m) => (
        <span key={m.level} style={{
          width: 8, height: 5, borderRadius: 1,
          background: m.level <= current ? maturityColor(current) : "transparent",
          border: `1px solid ${m.level <= current ? maturityColor(current) : m.level <= required ? "#ef4444" : "#1a2d42"}`,
        }} />
      ))}
    </span>
  );
}

function SidePanel({ scenario, report, fidelity, onCopy, copied, showMarkdown, onToggleMarkdown, markdown }) {
  const outcome = scenario.outcomes.find((o) => o.id === report.outcome);
  return (
    <div style={S.side}>
      <div style={S.sideHeader}>
        <span style={{ fontSize: 8, letterSpacing: ".16em", color: "#1e3a5f" }}>ACHIEVABLE TODAY</span>
        <span style={{ fontSize: 8, color: "#2d4460" }}>{report.bottlenecks.length} gap{report.bottlenecks.length === 1 ? "" : "s"}</span>
      </div>
      <div style={{ padding: "10px 14px 4px", flexShrink: 0 }}>
        <Gauge fidelity={fidelity} />
        <div style={{ fontSize: 8.5, color: "#3d5878", lineHeight: 1.5, marginTop: 4 }}>{outcome.description}</div>
      </div>

      <div style={{ ...S.sideHeader, borderTop: "1px solid #0d1e30" }}>
        <span style={{ fontSize: 8, letterSpacing: ".16em", color: "#1e3a5f" }}>WHAT TO FUND FIRST</span>
        <span style={{ fontSize: 8, color: "#2d4460" }}>worst first</span>
      </div>
      <div style={{ flex: 1, overflowY: "auto" }}>
        {report.bottlenecks.length === 0 && (
          <div style={{ padding: "14px", fontSize: 9, color: EDGE_STATE.satisfied.color }}>
            ● Every required domain meets its floor. Nothing to fund for this outcome.
          </div>
        )}
        {report.bottlenecks.map((b, i) => {
          const st = EDGE_STATE[b.state];
          return (
            <div key={b.domain} className="ev" style={{ padding: "8px 14px", borderBottom: "1px solid #040a12", borderLeft: `2px solid ${st.color}` }}>
              <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
                <span style={{ fontSize: 9, color: "#1e3040" }}>{i + 1}</span>
                <span style={{ fontSize: 10, fontWeight: 600, color: "#c7d2e0", flex: 1 }}>{b.domain_name}</span>
                <span style={{ fontSize: 7, letterSpacing: ".1em", color: st.color, border: `1px solid ${st.color}55`, padding: "1px 5px", borderRadius: 2 }}>
                  {b.criticality.toUpperCase()}
                </span>
              </div>
              <div style={{ fontSize: 8.5, color: "#3d5878", marginTop: 3, display: "flex", alignItems: "center", gap: 6 }}>
                <Pips current={b.current} required={b.required} />
                <span style={{ color: maturityColor(b.current) }}>{maturityLabel(b.current)}</span>
                <span style={{ color: "#1e3040" }}>→ needs</span>
                <span style={{ color: maturityColor(b.required) }}>{maturityLabel(b.required)}</span>
                <span style={{ color: st.color }}>+{b.gap}</span>
              </div>
              <div style={{ fontSize: 8, color: "#2d4460", marginTop: 3 }}>
                owner <span style={{ color: "#7a8fa8" }}>{b.owner}</span>
              </div>
              <div style={{ fontSize: 8, color: "#1e3040", marginTop: 2 }}>
                lives in {b.sources.map((id) => scenario.sources.find((s) => s.id === id)).filter(Boolean)
                  .map((s) => `${s.name} (${s.kind})`).join(", ") || "nothing recorded"}
              </div>
            </div>
          );
        })}
      </div>

      <div style={{ borderTop: "1px solid #0d1e30", padding: "8px 14px", display: "flex", gap: 6, flexShrink: 0 }}>
        <button onClick={onCopy} style={S.btn}>{copied ? "COPIED ✓" : "COPY GAP SUMMARY · MD"}</button>
        <button onClick={onToggleMarkdown} style={{ ...S.btn, color: "#3d5878", borderColor: "#0d1e30" }}>{showMarkdown ? "HIDE" : "VIEW"}</button>
      </div>
      {showMarkdown && (
        <textarea readOnly value={markdown} style={S.markdown} onFocus={(e) => e.target.select()} />
      )}
    </div>
  );
}

// ═══════════════════════════════════════════════════════════════════════════
// WHAT-IF BAR · one maturity slider per domain in the trace
// ═══════════════════════════════════════════════════════════════════════════

function WhatIfBar({ domains, report, overrides, onChange, onReset }) {
  const changed = Object.keys(overrides).length;
  return (
    <div style={S.whatIf}>
      <div style={{ display: "flex", flexDirection: "column", justifyContent: "center", padding: "0 14px", borderRight: "1px solid #0d1e30", flexShrink: 0, minWidth: 120 }}>
        <div style={{ fontSize: 7, color: "#1e3040", letterSpacing: ".16em" }}>WHAT-IF</div>
        <div style={{ fontSize: 8.5, color: "#3d5878", marginTop: 2, lineHeight: 1.4 }}>Drag a domain up the ladder and watch the trace re-run.</div>
        <button onClick={onReset} disabled={!changed} style={{ ...S.btn, flex: "0 0 auto", alignSelf: "flex-start", marginTop: 5, opacity: changed ? 1 : 0.35, padding: "3px 8px" }}>
          RESET {changed ? `(${changed})` : ""}
        </button>
      </div>
      <div style={{ display: "flex", flex: 1, overflowX: "auto", gap: 0 }}>
        {domains.map((d) => {
          const state = domainState(report, d.id);
          const st = EDGE_STATE[state];
          const base = domainById(BASE_SCENARIO, d.id).maturity;
          const changedHere = overrides[d.id] != null;
          return (
            <div key={d.id} style={{ minWidth: 168, padding: "7px 12px", borderRight: "1px solid #0d1e30", display: "flex", flexDirection: "column", gap: 3 }}>
              <div style={{ display: "flex", alignItems: "center", gap: 5 }}>
                <span style={{ width: 6, height: 6, borderRadius: "50%", background: st.color, boxShadow: `0 0 4px ${st.color}`, flexShrink: 0 }} />
                <span style={{ fontSize: 9, fontWeight: 600, color: "#c7d2e0", whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>{d.name}</span>
              </div>
              <input type="range" min={0} max={4} step={1} value={d.maturity} className="rt-slider" style={{ "--c": maturityColor(d.maturity), marginBottom: 3 }}
                onChange={(e) => onChange(d.id, Number(e.target.value))} aria-label={`${d.name} maturity`} />
              <div style={{ fontSize: 8, display: "flex", justifyContent: "space-between" }}>
                <span style={{ color: maturityColor(d.maturity) }}>{maturityLabel(d.maturity)}</span>
                <span style={{ color: changedHere ? ACCENT : "#1e3040" }}>{changedHere ? `was L${base}` : st.label.toLowerCase()}</span>
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}

// ═══════════════════════════════════════════════════════════════════════════
// STYLES
// ═══════════════════════════════════════════════════════════════════════════

const CSS = `
  @keyframes rtPulse { 0%,100% { opacity: 1 } 50% { opacity: .55 } }
  .rt-pulse { animation: rtPulse 1.1s ease-in-out infinite; transform-box: fill-box; }
  @keyframes rtBurst { from { r: 3; opacity: .9 } to { r: 14; opacity: 0 } }
  .rt-burst { animation: rtBurst .6s ease-out forwards; }
  .rt-slider { -webkit-appearance: none; appearance: none; width: 100%; height: 3px; background: #0d1e30; border-radius: 2px; outline: none; cursor: pointer; }
  .rt-slider::-webkit-slider-thumb { -webkit-appearance: none; appearance: none; width: 11px; height: 11px; border-radius: 50%; background: var(--c); box-shadow: 0 0 6px var(--c); border: 1px solid #020408; }
  .rt-slider::-moz-range-thumb { width: 11px; height: 11px; border-radius: 50%; background: var(--c); box-shadow: 0 0 6px var(--c); border: 1px solid #020408; }
`;

const S = {
  root: {
    display: "flex", flexDirection: "column", height: "100%",
    background: "#020408", fontFamily: "'JetBrains Mono',monospace",
    color: "#94a3b8", fontSize: 12, overflow: "hidden",
  },
  header: {
    height: 52, flexShrink: 0, display: "flex", alignItems: "center",
    padding: "0 20px", gap: 16,
    background: "linear-gradient(180deg,#04080f,#020408)",
    borderBottom: "1px solid #0d1e30", position: "relative", zIndex: 10,
  },
  wordmarkMain: { fontFamily: "'Orbitron',sans-serif", fontSize: 15, fontWeight: 900, color: ACCENT, letterSpacing: ".1em" },
  wordmarkSub: { fontFamily: "'Orbitron',sans-serif", fontSize: 9, fontWeight: 700, color: "#1e3a5f", letterSpacing: ".18em", whiteSpace: "nowrap" },
  divider: { width: 1, height: 20, background: "#0d1e30", margin: "0 4px", flexShrink: 0 },
  outcomeBtn: {
    display: "flex", alignItems: "center", gap: 6, whiteSpace: "nowrap",
    background: "transparent", border: "1px solid #0d1e30", borderRadius: 3, cursor: "pointer",
    color: "#3d5878", fontSize: 8.5, letterSpacing: ".03em", padding: "4px 7px",
    fontFamily: "'JetBrains Mono',monospace",
  },
  outcomeBtnActive: { color: ACCENT, borderColor: ACCENT, background: "#06131c" },
  legend: {
    height: 26, flexShrink: 0, display: "flex", alignItems: "center", gap: 12,
    padding: "0 20px", borderBottom: "1px solid #0d1e30", background: "#03070d",
  },
  legendTitle: { fontSize: 7, color: "#1e3040", letterSpacing: ".16em" },
  body: { display: "flex", flex: 1, overflow: "hidden", position: "relative", zIndex: 1 },
  side: { width: 340, flexShrink: 0, borderLeft: "1px solid #0d1e30", display: "flex", flexDirection: "column", overflow: "hidden" },
  sideHeader: { padding: "9px 14px", flexShrink: 0, borderBottom: "1px solid #0d1e30", display: "flex", alignItems: "center", justifyContent: "space-between" },
  btn: {
    flex: 1, background: "transparent", border: `1px solid ${ACCENT}66`, borderRadius: 3, cursor: "pointer",
    color: ACCENT, fontSize: 8, letterSpacing: ".1em", padding: "6px 8px", fontFamily: "'JetBrains Mono',monospace",
  },
  markdown: {
    height: 170, flexShrink: 0, resize: "none", margin: "0 14px 10px", padding: 8,
    background: "#03070d", color: "#7a8fa8", border: "1px solid #0d1e30", borderRadius: 3,
    fontFamily: "'JetBrains Mono',monospace", fontSize: 8.5, lineHeight: 1.4,
  },
  whatIf: {
    height: 84, flexShrink: 0, borderTop: "1px solid #0d1e30", background: "#020408",
    display: "flex", position: "relative", zIndex: 10, overflow: "hidden",
  },
};
