/**
 * TransformationSimulator.jsx
 *
 * "Transformation" tab: the legacy-to-mesh migration journey for the three
 * manpower tracks, capability maturity across the four data mesh
 * principles, and a decision panel that lets an operator steer the next
 * simulated quarter.
 *
 * Connects to the meshed registry API for:
 *   - GET  /api/transform/state       — current snapshot
 *   - POST /api/transform/decisions   — queue a decision
 *   - POST /api/transform/advance     — apply queued decisions, advance a quarter
 *   - GET  /api/transform/events      — SSE stream of narrative events
 *
 * Falls back to an in-browser simulation (mirroring the backend's rules —
 * see meshed/transformation/engine.py) when the backend is unavailable, so
 * the tab is fully playable standalone.
 */

import { useCallback, useEffect, useRef, useState } from "react";

// ═══════════════════════════════════════════════════════════════════════════
// CONFIGURATION — mirrors meshed/transformation/{enums,seed,engine}.py
// ═══════════════════════════════════════════════════════════════════════════

const TRACKS = [
  { track: "personnel-lifecycle", name: "Personnel Legacy DB", target: "personnel-lifecycle" },
  { track: "position-management", name: "Position Management Spreadsheets", target: "position-management" },
  { track: "readiness-reporting", name: "Readiness Manual Reports", target: "readiness-reporting" },
];

const DIMENSIONS = [
  { id: "domain_ownership", label: "OWNERSHIP" },
  { id: "data_as_a_product", label: "PRODUCT" },
  { id: "self_serve_platform", label: "PLATFORM" },
  { id: "federated_governance", label: "GOVERNANCE" },
];

const STATUS_COLOR = {
  legacy: "#ef4444",
  dual_write: "#fbbf24",
  migrated: "#34d399",
  decommissioned: "#f472b6",
};

const STATUS_LABEL = {
  legacy: "LEGACY",
  dual_write: "DUAL-WRITE",
  migrated: "MIGRATED",
  decommissioned: "DECOMMISSIONED (RISKY)",
};

const DUAL_WRITE_MIN_QUARTERS = 2;
const SCORE_MIN = 0, SCORE_MAX = 5;
const clamp = (v) => Math.max(SCORE_MIN, Math.min(SCORE_MAX, v));

let _eventId = 0;

// ═══════════════════════════════════════════════════════════════════════════
// LOCAL SIMULATION FALLBACK — mirrors meshed/transformation/engine.py
// ═══════════════════════════════════════════════════════════════════════════

function initialLocalState() {
  const capability = {};
  TRACKS.forEach((t) => {
    capability[t.track] = Object.fromEntries(DIMENSIONS.map((d) => [d.id, 1.0]));
  });
  return {
    quarter: 0,
    legacy_systems: TRACKS.map((t) => ({
      track: t.track, name: t.name, target_data_product: t.target,
      status: "legacy", status_since_quarter: 0,
    })),
    capability,
    maturity_trend: [{ quarter: 0, maturity_index: 1.0 }],
    pending_decisions: [],
    decision_history: [],
  };
}

function queueLocalDecision(state, decisionType, target) {
  return {
    ...state,
    pending_decisions: [
      ...state.pending_decisions,
      { id: `${Date.now()}-${Math.random()}`, quarter: state.quarter + 1, decision_type: decisionType, target },
    ],
  };
}

/** Returns { state, events } — mirrors advance_quarter() in engine.py. */
function advanceLocalState(state) {
  const nextQ = state.quarter + 1;
  const systems = Object.fromEntries(state.legacy_systems.map((s) => [s.track, { ...s }]));
  const deltas = {};
  TRACKS.forEach((t) => { deltas[t.track] = Object.fromEntries(DIMENSIONS.map((d) => [d.id, 0])); });
  const bump = (track, dim, amt) => { deltas[track][dim] += amt; };
  const events = [];
  const emit = (eventType, message, track) => events.push({
    id: ++_eventId, quarter: nextQ, eventType, track: track || null, message,
    timestamp: new Date().toISOString(),
  });

  const pending = state.pending_decisions.filter((d) => d.quarter === nextQ);
  const stillPending = state.pending_decisions.filter((d) => d.quarter !== nextQ);
  const applied = [];

  for (const decision of pending) {
    const { decision_type: type, target } = decision;
    if (type === "migrate_track") {
      const ls = systems[target];
      if (ls && ls.status === "legacy") {
        ls.status = "dual_write"; ls.status_since_quarter = nextQ;
        bump(target, "domain_ownership", 0.5); bump(target, "data_as_a_product", 0.5);
        emit("wave_started", `${ls.name} begins dual-write to ${ls.target_data_product}`, target);
      } else {
        emit("decision_rejected", `Migrate ${target}: already past LEGACY status, decision skipped`, target);
      }
    } else if (type === "sunset_legacy") {
      const ls = systems[target];
      if (ls && ls.status === "dual_write") {
        ls.status = "migrated"; ls.status_since_quarter = nextQ;
        bump(target, "federated_governance", 0.5); bump(target, "data_as_a_product", 0.3);
        emit("system_decommissioned", `${ls.name} decommissioned cleanly — ${ls.target_data_product} is now sole source of truth`, target);
      } else if (ls && ls.status === "legacy") {
        ls.status = "decommissioned"; ls.status_since_quarter = nextQ;
        bump(target, "federated_governance", -1.0); bump(target, "data_as_a_product", -0.5);
        emit("maturity_regression", `${ls.name} decommissioned without dual-write — consumers lost lineage continuity`, target);
      } else {
        emit("decision_rejected", `Sunset ${target}: no eligible legacy system in LEGACY or DUAL_WRITE status`, target);
      }
    } else if (type === "invest_platform") {
      TRACKS.forEach((t) => bump(t.track, "self_serve_platform", 0.3));
      emit("decision_applied", "Platform investment lifts self-serve capability mesh-wide");
    } else if (type === "invest_product_teams") {
      TRACKS.forEach((t) => { bump(t.track, "domain_ownership", 0.2); bump(t.track, "data_as_a_product", 0.2); });
      emit("decision_applied", "Product team investment lifts domain ownership mesh-wide");
    }
    applied.push(decision);
  }

  // Dual-write auto-completion after the minimum period.
  Object.values(systems).forEach((ls) => {
    if (ls.status === "dual_write" && nextQ - ls.status_since_quarter >= DUAL_WRITE_MIN_QUARTERS) {
      ls.status = "migrated"; ls.status_since_quarter = nextQ;
      bump(ls.track, "federated_governance", 0.5); bump(ls.track, "data_as_a_product", 0.3);
      emit("system_decommissioned", `${ls.name} dual-write period complete — ${ls.target_data_product} cut over automatically`, ls.track);
    }
  });

  const capability = {};
  let sum = 0, count = 0;
  TRACKS.forEach((t) => {
    capability[t.track] = {};
    DIMENSIONS.forEach((d) => {
      const score = clamp(state.capability[t.track][d.id] + deltas[t.track][d.id]);
      capability[t.track][d.id] = score;
      sum += score; count += 1;
    });
  });

  const nextState = {
    quarter: nextQ,
    legacy_systems: TRACKS.map((t) => systems[t.track]),
    capability,
    maturity_trend: [...state.maturity_trend, { quarter: nextQ, maturity_index: +(sum / count).toFixed(2) }],
    pending_decisions: stillPending,
    decision_history: [...applied.map((d) => ({ ...d, applied: true })), ...state.decision_history].slice(0, 20),
  };

  return { state: nextState, events };
}

// ═══════════════════════════════════════════════════════════════════════════
// UTILS
// ═══════════════════════════════════════════════════════════════════════════

function formatTs(iso) {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "";
  return [d.getHours(), d.getMinutes(), d.getSeconds()].map((v) => String(v).padStart(2, "0")).join(":");
}

function orgMaturityIndex(capability) {
  let sum = 0, count = 0;
  Object.values(capability).forEach((dims) => {
    Object.values(dims).forEach((v) => { sum += v; count += 1; });
  });
  return count ? sum / count : 0;
}

function orgDimensionAverages(capability) {
  const totals = Object.fromEntries(DIMENSIONS.map((d) => [d.id, 0]));
  const tracks = Object.values(capability);
  tracks.forEach((dims) => DIMENSIONS.forEach((d) => { totals[d.id] += dims[d.id] || 0; }));
  const n = tracks.length || 1;
  return Object.fromEntries(DIMENSIONS.map((d) => [d.id, totals[d.id] / n]));
}

// ═══════════════════════════════════════════════════════════════════════════
// ROOT COMPONENT
// ═══════════════════════════════════════════════════════════════════════════

export default function TransformationSimulator() {
  const [state, setState] = useState(initialLocalState);
  const [events, setEvents] = useState([]);
  const [isLive, setIsLive] = useState(false);
  const [busy, setBusy] = useState(false);
  const liveRef = useRef(false);

  const pushEvents = useCallback((incoming) => {
    if (!incoming?.length) return;
    setEvents((prev) => [...incoming.slice().reverse(), ...prev].slice(0, 100));
  }, []);

  // ── Probe backend once on mount; use live mode if it answers ──────────
  useEffect(() => {
    let cancelled = false;

    async function probe() {
      try {
        const res = await fetch("/api/transform/state");
        if (!res.ok) throw new Error(`HTTP ${res.status}`);
        const data = await res.json();
        if (cancelled) return;
        liveRef.current = true;
        setIsLive(true);
        setState(data);
      } catch {
        // Backend unavailable — keep local simulation state.
      }
    }
    probe();
    return () => { cancelled = true; };
  }, []);

  // ── SSE event stream (live mode only) ──────────────────────────────────
  useEffect(() => {
    if (!isLive) return undefined;
    let alive = true;
    const es = new EventSource("/api/transform/events");
    es.onmessage = (e) => {
      if (!alive) return;
      try {
        const ev = JSON.parse(e.data);
        if (ev.eventType) pushEvents([ev]);
      } catch {
        // heartbeat or malformed — ignore
      }
    };
    es.onerror = () => { if (alive) es.close(); };
    return () => { alive = false; es.close(); };
  }, [isLive, pushEvents]);

  const queueDecision = useCallback(async (decisionType, target) => {
    if (busy) return;
    if (liveRef.current) {
      setBusy(true);
      try {
        const res = await fetch("/api/transform/decisions", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ decision_type: decisionType, target }),
        });
        if (res.ok) {
          const res2 = await fetch("/api/transform/state");
          if (res2.ok) setState(await res2.json());
        }
      } catch {
        // Connection dropped mid-session — leave state as-is.
      } finally {
        setBusy(false);
      }
    } else {
      setState((s) => queueLocalDecision(s, decisionType, target));
    }
  }, [busy]);

  const advance = useCallback(async () => {
    if (busy) return;
    setBusy(true);
    try {
      if (liveRef.current) {
        const res = await fetch("/api/transform/advance", { method: "POST" });
        if (res.ok) setState(await res.json());
      } else {
        // Compute the transition from the current state directly rather than
        // inside the setState updater — an updater must be pure, and React
        // 18 StrictMode double-invokes it in dev, which would double-emit
        // events if pushEvents (itself a setState) ran inside it.
        const { state: next, events: newEvents } = advanceLocalState(state);
        setState(next);
        pushEvents(newEvents);
      }
    } catch {
      // Connection dropped mid-session — leave state as-is.
    } finally {
      setBusy(false);
    }
  }, [busy, pushEvents, state]);

  const maturityIndex = orgMaturityIndex(state.capability);
  const dimAverages = orgDimensionAverages(state.capability);

  return (
    <div style={S.root}>
      <TransformHeader
        quarter={state.quarter}
        maturityIndex={maturityIndex}
        isLive={isLive}
        busy={busy}
        onAdvance={advance}
        pendingCount={state.pending_decisions.length}
      />
      <div style={S.body}>
        <div style={S.leftCol}>
          <MigrationTimeline legacySystems={state.legacy_systems} currentQuarter={state.quarter} />
          <CapabilityRadar dimAverages={dimAverages} maturityTrend={state.maturity_trend} />
        </div>
        <div style={S.rightCol}>
          <DecisionPanel
            legacySystems={state.legacy_systems}
            pendingDecisions={state.pending_decisions}
            quarter={state.quarter}
            busy={busy}
            onQueue={queueDecision}
          />
          <EventTicker events={events} />
        </div>
      </div>
    </div>
  );
}

// ═══════════════════════════════════════════════════════════════════════════
// HEADER
// ═══════════════════════════════════════════════════════════════════════════

function TransformHeader({ quarter, maturityIndex, isLive, busy, onAdvance, pendingCount }) {
  return (
    <div style={S.header}>
      <div style={{ display: "flex", alignItems: "baseline", gap: 8 }}>
        <span style={S.wordmarkMain}>TRANSFORMATION</span>
        <span style={S.wordmarkSub}>PROGRAM</span>
      </div>

      <div style={{ display: "flex", alignItems: "center", gap: 5 }}>
        <div className="live" style={{
          width: 7, height: 7, borderRadius: "50%",
          background: isLive ? "#22c55e" : "#fbbf24", boxShadow: `0 0 6px ${isLive ? "#22c55e" : "#fbbf24"}`,
        }} />
        <span style={{ fontSize: 9, color: isLive ? "#22c55e" : "#fbbf24", letterSpacing: ".12em" }}>
          {isLive ? "LIVE" : "SIM"}
        </span>
      </div>

      <div style={S.divider} />

      <div style={{ textAlign: "left" }}>
        <div style={S.kpiLabel}>QUARTER</div>
        <div style={{ ...S.kpiVal, color: "#22d3ee" }}>Q{quarter}</div>
      </div>
      <div style={{ textAlign: "left" }}>
        <div style={S.kpiLabel}>MATURITY INDEX</div>
        <div style={{ ...S.kpiVal, color: "#34d399" }}>{maturityIndex.toFixed(2)} / 5.00</div>
      </div>
      <div style={{ textAlign: "left" }}>
        <div style={S.kpiLabel}>QUEUED DECISIONS</div>
        <div style={{ ...S.kpiVal, color: pendingCount ? "#fbbf24" : "#334155" }}>{pendingCount}</div>
      </div>

      <div style={{ flex: 1 }} />

      <button onClick={onAdvance} disabled={busy} style={S.advanceButton}>
        {busy ? "ADVANCING…" : "ADVANCE QUARTER →"}
      </button>
    </div>
  );
}

// ═══════════════════════════════════════════════════════════════════════════
// MIGRATION TIMELINE
// ═══════════════════════════════════════════════════════════════════════════

function MigrationTimeline({ legacySystems, currentQuarter }) {
  return (
    <div style={S.panel}>
      <div style={S.panelHeader}>
        <span style={S.panelTitle}>MIGRATION TIMELINE</span>
        <span style={S.panelSub}>legacy → mesh, by track</span>
      </div>
      <div style={{ padding: "12px 16px", display: "flex", flexDirection: "column", gap: 14 }}>
        {legacySystems.map((ls) => {
          const color = STATUS_COLOR[ls.status];
          const quartersInStatus = currentQuarter - ls.status_since_quarter;
          return (
            <div key={ls.track}>
              <div style={{ display: "flex", justifyContent: "space-between", alignItems: "baseline", marginBottom: 4 }}>
                <span style={{ fontSize: 11, color: "#c9d6e3" }}>{ls.name}</span>
                <span style={{ fontSize: 8, color, letterSpacing: ".08em" }}>{STATUS_LABEL[ls.status]}</span>
              </div>
              <div style={S.timelineTrack}>
                {["legacy", "dual_write", "migrated"].map((stage, i) => {
                  const stageIdx = { legacy: 0, dual_write: 1, migrated: 2, decommissioned: 2 }[ls.status];
                  const reached = i <= stageIdx;
                  const isRisky = ls.status === "decommissioned" && i === 2;
                  return (
                    <div
                      key={stage}
                      style={{
                        flex: 1, height: 6, borderRadius: 3,
                        background: isRisky ? STATUS_COLOR.decommissioned : reached ? color : "#0d1e30",
                        opacity: reached ? 1 : 0.5,
                        transition: "background 300ms",
                      }}
                    />
                  );
                })}
              </div>
              <div style={{ fontSize: 8, color: "#3d5878", marginTop: 3 }}>
                → {ls.target_data_product}
                {ls.status === "dual_write" && ` · dual-write for ${quartersInStatus}q`}
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}

// ═══════════════════════════════════════════════════════════════════════════
// CAPABILITY RADAR
// ═══════════════════════════════════════════════════════════════════════════

function radarPoint(cx, cy, r, fraction, angleIdx, total) {
  const angle = -Math.PI / 2 + (angleIdx / total) * 2 * Math.PI;
  return [cx + r * fraction * Math.cos(angle), cy + r * fraction * Math.sin(angle)];
}

function CapabilityRadar({ dimAverages, maturityTrend }) {
  const width = 320, height = 260, cx = 160, cy = height / 2, r = 76;
  const n = DIMENSIONS.length;

  const polygon = DIMENSIONS.map((d, i) => radarPoint(cx, cy, r, dimAverages[d.id] / SCORE_MAX, i, n))
    .map(([x, y]) => `${x.toFixed(1)},${y.toFixed(1)}`)
    .join(" ");

  const rings = [0.25, 0.5, 0.75, 1.0];
  const trendMax = Math.max(1, ...maturityTrend.map((e) => e.maturity_index));
  const sparkW = 150, sparkH = 40;

  return (
    <div style={{ ...S.panel, flex: 1, minHeight: 0, display: "flex", flexDirection: "column" }}>
      <div style={S.panelHeader}>
        <span style={S.panelTitle}>CAPABILITY MATURITY</span>
        <span style={S.panelSub}>org-wide average, four principles</span>
      </div>
      <div style={{ display: "flex", flex: 1, alignItems: "center", justifyContent: "center", padding: "10px 16px", gap: 32, overflow: "auto" }}>
        <svg width={width} height={height} viewBox={`0 0 ${width} ${height}`} style={{ flexShrink: 0 }}>
          {rings.map((frac) => (
            <polygon
              key={frac}
              points={DIMENSIONS.map((_, i) => radarPoint(cx, cy, r, frac, i, n)).map(([x, y]) => `${x.toFixed(1)},${y.toFixed(1)}`).join(" ")}
              fill="none" stroke="#0d1e30" strokeWidth="1"
            />
          ))}
          {DIMENSIONS.map((d, i) => {
            const [x, y] = radarPoint(cx, cy, r, 1, i, n);
            return <line key={d.id} x1={cx} y1={cy} x2={x} y2={y} stroke="#0d1e30" strokeWidth="1" />;
          })}
          <polygon points={polygon} fill="#22d3ee22" stroke="#22d3ee" strokeWidth="1.5" />
          {DIMENSIONS.map((d, i) => {
            const [x, y] = radarPoint(cx, cy, r + 26, 1, i, n);
            const anchor = Math.abs(x - cx) < 4 ? "middle" : x > cx ? "start" : "end";
            return (
              <text key={d.id} x={x} y={y} fill="#3d5878" fontSize="7.5" textAnchor={anchor} dominantBaseline="middle" fontFamily="'JetBrains Mono',monospace">
                {d.label}
              </text>
            );
          })}
        </svg>

        <div style={{ display: "flex", flexDirection: "column", gap: 12, minWidth: 140 }}>
          {DIMENSIONS.map((d) => (
            <div key={d.id}>
              <div style={{ fontSize: 7, color: "#3d5878", letterSpacing: ".08em" }}>{d.label}</div>
              <div style={{ fontSize: 12, color: "#c9d6e3" }}>{dimAverages[d.id].toFixed(2)}</div>
            </div>
          ))}
          <svg width={sparkW} height={sparkH} viewBox={`0 0 ${sparkW} ${sparkH}`}>
            <polyline
              fill="none" stroke="#34d399" strokeWidth="1.5"
              points={maturityTrend.map((e, i) => {
                const x = (i / Math.max(1, maturityTrend.length - 1)) * sparkW;
                const y = sparkH - (e.maturity_index / trendMax) * sparkH;
                return `${x.toFixed(1)},${y.toFixed(1)}`;
              }).join(" ")}
            />
          </svg>
          <div style={{ fontSize: 7, color: "#3d5878" }}>maturity trend, {maturityTrend.length}q</div>
        </div>
      </div>
    </div>
  );
}

// ═══════════════════════════════════════════════════════════════════════════
// DECISION PANEL
// ═══════════════════════════════════════════════════════════════════════════

function DecisionPanel({ legacySystems, pendingDecisions, quarter, busy, onQueue }) {
  const pendingTargets = new Set(pendingDecisions.map((d) => d.target));

  return (
    <div style={S.panel}>
      <div style={S.panelHeader}>
        <span style={S.panelTitle}>DECISIONS</span>
        <span style={S.panelSub}>queued for Q{quarter + 1}</span>
      </div>
      <div style={{ padding: "10px 14px", display: "flex", flexDirection: "column", gap: 8 }}>
        {legacySystems.map((ls) => {
          const queued = pendingTargets.has(ls.track);
          const canMigrate = ls.status === "legacy";
          const canSunset = ls.status === "dual_write" || ls.status === "legacy";
          return (
            <div key={ls.track} style={S.decisionRow}>
              <span style={{ fontSize: 9.5, color: "#94a3b8", flex: 1 }}>{ls.name}</span>
              <button
                disabled={busy || queued || !canMigrate}
                onClick={() => onQueue("migrate_track", ls.track)}
                style={{ ...S.decisionBtn, opacity: canMigrate && !queued ? 1 : 0.35 }}
                title="Begin dual-write"
              >
                MIGRATE
              </button>
              <button
                disabled={busy || queued || !canSunset}
                onClick={() => onQueue("sunset_legacy", ls.track)}
                style={{
                  ...S.decisionBtn,
                  opacity: canSunset && !queued ? 1 : 0.35,
                  borderColor: ls.status === "legacy" ? "#7f1d1d" : "#0d1e30",
                  color: ls.status === "legacy" ? "#f87171" : "#94a3b8",
                }}
                title={ls.status === "legacy" ? "Risky — skips dual-write" : "Clean cutover"}
              >
                SUNSET
              </button>
            </div>
          );
        })}

        <div style={{ height: 1, background: "#0d1e30", margin: "4px 0" }} />

        <div style={S.decisionRow}>
          <span style={{ fontSize: 9.5, color: "#94a3b8", flex: 1 }}>Platform investment</span>
          <button
            disabled={busy || pendingTargets.has("platform")}
            onClick={() => onQueue("invest_platform", "platform")}
            style={{ ...S.decisionBtn, opacity: pendingTargets.has("platform") ? 0.35 : 1 }}
          >
            INVEST
          </button>
        </div>
        <div style={S.decisionRow}>
          <span style={{ fontSize: 9.5, color: "#94a3b8", flex: 1 }}>Product team investment</span>
          <button
            disabled={busy || pendingTargets.has("product_teams")}
            onClick={() => onQueue("invest_product_teams", "product_teams")}
            style={{ ...S.decisionBtn, opacity: pendingTargets.has("product_teams") ? 0.35 : 1 }}
          >
            INVEST
          </button>
        </div>

        {pendingDecisions.length > 0 && (
          <div style={{ marginTop: 6, fontSize: 8, color: "#fbbf24" }}>
            {pendingDecisions.length} decision{pendingDecisions.length > 1 ? "s" : ""} queued — click Advance Quarter to apply.
          </div>
        )}
      </div>
    </div>
  );
}

// ═══════════════════════════════════════════════════════════════════════════
// EVENT TICKER
// ═══════════════════════════════════════════════════════════════════════════

const EVENT_COLOR = {
  wave_started: "#22d3ee",
  system_decommissioned: "#34d399",
  maturity_regression: "#f87171",
  decision_applied: "#c084fc",
  decision_rejected: "#3d5878",
};

function EventTicker({ events }) {
  return (
    <div style={{ ...S.panel, flex: 1, minHeight: 0, display: "flex", flexDirection: "column" }}>
      <div style={S.panelHeader}>
        <span style={S.panelTitle}>EVENT LOG</span>
        <span style={S.panelSub}>{events.length} events</span>
      </div>
      <div style={{ flex: 1, overflow: "auto", padding: "6px 12px" }}>
        {events.length === 0 && (
          <div style={{ fontSize: 9.5, color: "#3d5878", padding: "10px 4px" }}>
            No transformation events yet — queue a decision and advance the quarter.
          </div>
        )}
        {events.map((ev) => (
          <div key={ev.id} className="ev" style={{ padding: "6px 4px", borderBottom: "1px solid #0a1420" }}>
            <div style={{ display: "flex", justifyContent: "space-between" }}>
              <span style={{ fontSize: 8, color: EVENT_COLOR[ev.eventType] || "#94a3b8", letterSpacing: ".06em" }}>
                Q{ev.quarter} · {ev.eventType.replace(/_/g, " ").toUpperCase()}
              </span>
              <span style={{ fontSize: 8, color: "#334155" }}>{formatTs(ev.timestamp)}</span>
            </div>
            <div style={{ fontSize: 9.5, color: "#c9d6e3", marginTop: 2 }}>{ev.message}</div>
          </div>
        ))}
      </div>
    </div>
  );
}

// ═══════════════════════════════════════════════════════════════════════════
// STYLES
// ═══════════════════════════════════════════════════════════════════════════

const S = {
  root: {
    display: "flex", flexDirection: "column", height: "100%",
    background: "#020408", fontFamily: "'JetBrains Mono',monospace",
    color: "#94a3b8", fontSize: 12, overflow: "hidden",
  },
  header: {
    height: 52, flexShrink: 0, display: "flex", alignItems: "center",
    padding: "0 20px", gap: 20,
    background: "linear-gradient(180deg,#04080f,#020408)",
    borderBottom: "1px solid #0d1e30",
  },
  wordmarkMain: {
    fontFamily: "'Orbitron',sans-serif", fontSize: 13, fontWeight: 900,
    color: "#22d3ee", letterSpacing: ".08em",
  },
  wordmarkSub: {
    fontFamily: "'Orbitron',sans-serif", fontSize: 9, fontWeight: 700,
    color: "#1e3a5f", letterSpacing: ".18em",
  },
  divider: { width: 1, height: 20, background: "#0d1e30", margin: "0 4px" },
  kpiLabel: { fontSize: 7, color: "#1e3040", letterSpacing: ".14em", marginBottom: 2 },
  kpiVal: { fontSize: 14, fontWeight: 600 },
  advanceButton: {
    background: "#0a1a26", border: "1px solid #22d3ee", color: "#22d3ee",
    fontFamily: "'JetBrains Mono',monospace", fontSize: 10, letterSpacing: ".08em",
    padding: "8px 16px", borderRadius: 3, cursor: "pointer",
  },
  body: { display: "flex", flex: 1, overflow: "hidden" },
  leftCol: { flex: 1, display: "flex", flexDirection: "column", gap: 1, overflow: "hidden", minWidth: 0 },
  rightCol: { width: 340, flexShrink: 0, display: "flex", flexDirection: "column", gap: 1, borderLeft: "1px solid #0d1e30", overflow: "hidden" },
  panel: { borderBottom: "1px solid #0d1e30", overflow: "hidden", display: "flex", flexDirection: "column" },
  panelHeader: {
    padding: "8px 16px", flexShrink: 0, borderBottom: "1px solid #0d1e30",
    display: "flex", alignItems: "baseline", gap: 8,
  },
  panelTitle: { fontSize: 9, color: "#c9d6e3", letterSpacing: ".1em" },
  panelSub: { fontSize: 8, color: "#3d5878" },
  timelineTrack: { display: "flex", gap: 2 },
  decisionRow: { display: "flex", alignItems: "center", gap: 6 },
  decisionBtn: {
    background: "transparent", border: "1px solid #0d1e30", color: "#94a3b8",
    fontFamily: "'JetBrains Mono',monospace", fontSize: 8, letterSpacing: ".06em",
    padding: "4px 8px", borderRadius: 2, cursor: "pointer", flexShrink: 0,
  },
};
