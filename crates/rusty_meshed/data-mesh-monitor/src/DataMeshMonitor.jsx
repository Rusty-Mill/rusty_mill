/**
 * DataMeshMonitor.jsx
 *
 * Full-screen ops-center dashboard for monitoring the meshed data mesh platform.
 * Topology graph (SVG) + live event stream + bottom metrics bar.
 *
 * Connects to the meshed registry API for:
 *   - GET /api/mesh/topology  — dynamic nodes/edges from registered data products
 *   - GET /api/mesh/events    — SSE stream of lineage records & violations
 *   - GET /api/mesh/metrics   — aggregate platform metrics
 *
 * Falls back to simulation mode when the backend is unavailable.
 */

import { useState, useEffect, useRef, useCallback } from "react";

// ═══════════════════════════════════════════════════════════════════════════
// CONFIGURATION
// ═══════════════════════════════════════════════════════════════════════════

/** Color palette per node type */
const C = {
  producer:  { color: "#22d3ee", dim: "#0e4a5a" },
  broker:    { color: "#fbbf24", dim: "#5a3d00" },
  processor: { color: "#c084fc", dim: "#3d1a5e" },
  consumer:  { color: "#34d399", dim: "#0a3d27" },
};

/** Default topology for simulation fallback */
const DEFAULT_NODES = [
  { id: "p1", type: "producer",  label: "Orders API",   sub: "REST · gRPC",    x: 90,  y: 95  },
  { id: "p2", type: "producer",  label: "Inventory",    sub: "CDC · Debezium", x: 90,  y: 235 },
  { id: "p3", type: "producer",  label: "Clickstream",  sub: "JS SDK · Kafka", x: 90,  y: 375 },
  { id: "b1", type: "broker",    label: "NATS",         sub: "JetStream · 3n", x: 295, y: 165 },
  { id: "b2", type: "broker",    label: "Kafka Bus",    sub: "12 topics · 3p", x: 295, y: 315 },
  { id: "r1", type: "processor", label: "Enricher",     sub: "Python · 4w",    x: 500, y: 95  },
  { id: "r2", type: "processor", label: "Validator",    sub: "Rust · 2w",      x: 500, y: 245 },
  { id: "r3", type: "processor", label: "Aggregator",   sub: "Flink · 6t",     x: 500, y: 395 },
  { id: "c1", type: "consumer",  label: "Warehouse",    sub: "PostgreSQL",      x: 705, y: 75  },
  { id: "c2", type: "consumer",  label: "Fulfillment",  sub: "Microservice",    x: 705, y: 200 },
  { id: "c3", type: "consumer",  label: "Reporting",    sub: "ClickHouse",      x: 705, y: 320 },
  { id: "c4", type: "consumer",  label: "DLQ · Alerts", sub: "PagerDuty",      x: 705, y: 440 },
];

const DEFAULT_EDGES = [
  { id: "e1",  from: "p1", to: "b1" },
  { id: "e2",  from: "p2", to: "b1" },
  { id: "e3",  from: "p2", to: "b2" },
  { id: "e4",  from: "p3", to: "b2" },
  { id: "e5",  from: "b1", to: "r1" },
  { id: "e6",  from: "b1", to: "r2" },
  { id: "e7",  from: "b2", to: "r2" },
  { id: "e8",  from: "b2", to: "r3" },
  { id: "e9",  from: "r1", to: "c1" },
  { id: "e10", from: "r1", to: "c2" },
  { id: "e11", from: "r2", to: "c2" },
  { id: "e12", from: "r2", to: "c3" },
  { id: "e13", from: "r3", to: "c3" },
  { id: "e14", from: "r3", to: "c4" },
];

/** Simulated event type names (fallback mode) */
const SIM_EVENT_TYPES = [
  "order.placed", "order.fulfilled", "inventory.delta", "user.session.start",
  "payment.captured", "alert.critical", "schema.validated", "record.enriched",
  "window.computed", "dlq.routed", "consumer.ack", "stock.reserved",
  "order.cancelled", "click.converted", "metric.flush",
];

// ═══════════════════════════════════════════════════════════════════════════
// UTILS
// ═══════════════════════════════════════════════════════════════════════════

const pick = (arr) => arr[Math.floor(Math.random() * arr.length)];
const rand = (lo, hi) => Math.random() * (hi - lo) + lo;

/** Cubic bezier SVG path between two node centres */
function bezier(a, b) {
  const dx = (b.x - a.x) * 0.42;
  return `M${a.x},${a.y} C${a.x + dx},${a.y} ${b.x - dx},${b.y} ${b.x},${b.y}`;
}

let _particleId = 0;
let _eventId = 0;

// ═══════════════════════════════════════════════════════════════════════════
// ROOT COMPONENT
// ═══════════════════════════════════════════════════════════════════════════

export default function DataMeshMonitor() {
  const [nodes, setNodes]               = useState(DEFAULT_NODES);
  const [edges, setEdges]               = useState(DEFAULT_EDGES);
  const [particles, setParticles]       = useState([]);
  const [events, setEvents]             = useState([]);
  const [activeNodes, setActiveNodes]   = useState(new Set());
  const [isLive, setIsLive]             = useState(false);
  const [platformMetrics, setPlatformMetrics] = useState(null);
  const [metrics, setMetrics]           = useState({
    total: 0, tps: 0, latency: 24, errRate: 0.8, flows: 14, dlq: 0,
  });
  const [sparkline, setSparkline] = useState(Array(36).fill(0));
  const tpsRef = useRef(0);
  const nodesRef = useRef(nodes);
  const edgesRef = useRef(edges);

  // Keep refs in sync with state
  useEffect(() => { nodesRef.current = nodes; }, [nodes]);
  useEffect(() => { edgesRef.current = edges; }, [edges]);

  // Helper to find a node by ID from current state
  const nodeById = useCallback((id) => nodesRef.current.find((n) => n.id === id), []);

  // ── Handle an incoming event (from SSE or simulation) ─────────────────
  const handleEvent = useCallback((ev) => {
    const currentNodes = nodesRef.current;
    const currentEdges = edgesRef.current;

    // Find source and target nodes
    let fromNode = currentNodes.find((n) => n.label === ev.from || n.id === ev.from);
    let toNode = currentNodes.find((n) => n.label === ev.to || n.id === ev.to);

    if (!fromNode || !toNode) return;

    // Find edge or create a synthetic path
    const edge = currentEdges.find(
      (e) => (e.from === fromNode.id && e.to === toNode.id)
    );

    const dur = rand(800, 2000);
    const pid = ++_particleId;
    const path = bezier(fromNode, toNode);

    // Particle
    setParticles((p) => [
      ...p.slice(-100),
      { id: pid, path, color: C[fromNode.type]?.color || "#22d3ee", dur, start: Date.now(), isErr: ev.isErr },
    ]);

    // Event log entry
    const now = new Date();
    const ts = [now.getHours(), now.getMinutes(), now.getSeconds()]
      .map((v) => String(v).padStart(2, "0")).join(":")
      + `.${String(now.getMilliseconds()).padStart(3, "0")}`;

    setEvents((prev) => [
      {
        id: ++_eventId,
        type: ev.type,
        ts,
        isErr: ev.isErr || false,
        lat: ev.lat || 0,
        kb: ev.kb || 0,
        from: fromNode.label,
        fromType: fromNode.type,
        to: toNode.label,
      },
      ...prev.slice(0, 299),
    ]);

    // Node activation flash
    setActiveNodes((s) => new Set([...s, fromNode.id, toNode.id]));
    setTimeout(() => {
      setActiveNodes((s) => {
        const n = new Set(s);
        n.delete(fromNode.id);
        n.delete(toNode.id);
        return n;
      });
    }, 480);

    tpsRef.current++;
  }, []);

  // ── Fetch topology from registry API ──────────────────────────────────
  useEffect(() => {
    let cancelled = false;

    async function fetchTopology() {
      try {
        const res = await fetch("/api/mesh/topology");
        if (!res.ok) throw new Error(`HTTP ${res.status}`);
        const data = await res.json();
        if (!cancelled && data.nodes?.length > 0) {
          setNodes(data.nodes);
          setEdges(data.edges || []);
        }
      } catch {
        // Backend unavailable — keep default topology
      }
    }

    fetchTopology();
    // Refresh topology every 30 seconds
    const iv = setInterval(fetchTopology, 30000);
    return () => { cancelled = true; clearInterval(iv); };
  }, []);

  // ── Fetch platform metrics from registry API ─────────────────────────
  useEffect(() => {
    async function fetchMetrics() {
      try {
        const res = await fetch("/api/mesh/metrics");
        if (!res.ok) return;
        const data = await res.json();
        setPlatformMetrics(data);
      } catch {
        // Backend unavailable
      }
    }

    fetchMetrics();
    const iv = setInterval(fetchMetrics, 5000);
    return () => clearInterval(iv);
  }, []);

  // ── SSE connection with simulation fallback ───────────────────────────
  useEffect(() => {
    let alive = true;
    let es = null;

    function startSSE() {
      es = new EventSource("/api/mesh/events");

      es.onopen = () => {
        if (alive) setIsLive(true);
      };

      es.onmessage = (e) => {
        if (!alive) return;
        try {
          const ev = JSON.parse(e.data);
          handleEvent(ev);
        } catch {
          // Ignore malformed events
        }
      };

      es.onerror = () => {
        if (!alive) return;
        es.close();
        setIsLive(false);
        // Fall back to simulation after SSE failure
        startSimulation();
      };
    }

    function startSimulation() {
      if (!alive) return;

      const fire = () => {
        if (!alive) return;
        const currentEdges = edgesRef.current;
        const currentNodes = nodesRef.current;
        if (currentEdges.length === 0 || currentNodes.length === 0) {
          setTimeout(fire, 500);
          return;
        }

        const edge = pick(currentEdges);
        const from = currentNodes.find((n) => n.id === edge.from);
        const to = currentNodes.find((n) => n.id === edge.to);
        if (!from || !to) {
          setTimeout(fire, rand(120, 680));
          return;
        }

        const type = pick(SIM_EVENT_TYPES);
        const isErr = Math.random() < 0.04;
        const lat = Math.floor(rand(4, 190));
        const kb = +rand(0.1, 18).toFixed(1);

        handleEvent({ type, from: from.id, to: to.id, isErr, lat, kb });
        setTimeout(fire, rand(120, 680));
      };

      // Spawn 3 concurrent event streams with small offsets
      [0, 80, 200].forEach((d) => setTimeout(fire, d));
    }

    // Try SSE first, fall back to simulation
    try {
      startSSE();
    } catch {
      startSimulation();
    }

    return () => {
      alive = false;
      if (es) es.close();
    };
  }, [handleEvent]);

  // ── Metrics + particle cleanup (1 s tick) ────────────────────────────
  useEffect(() => {
    const iv = setInterval(() => {
      const tps = tpsRef.current;
      tpsRef.current = 0;
      const now = Date.now();

      setParticles((p) => p.filter((x) => now - x.start < x.dur + 300));
      setMetrics((prev) => ({
        total:   prev.total + tps,
        tps,
        latency: Math.floor(rand(10, 58)),
        errRate: +rand(0.1, 3.8).toFixed(1),
        flows:   edgesRef.current.length - Math.floor(rand(0, 2)),
        dlq:     prev.dlq + (Math.random() < 0.12 ? 1 : 0),
      }));
      setSparkline((s) => [...s.slice(1), tps]);
    }, 1000);
    return () => clearInterval(iv);
  }, []);

  const errColor = metrics.errRate > 2 ? "#ef4444" : "#fbbf24";
  const totalFlows = edges.length;

  return (
    <>
      <style>{GLOBAL_CSS}</style>

      <div style={S.root}>
        {/* Scanline overlay */}
        <div style={S.scanline} />

        <Header
          metrics={metrics}
          errColor={errColor}
          isLive={isLive}
          totalFlows={totalFlows}
          platformMetrics={platformMetrics}
        />

        <div style={S.body}>
          <TopologyPanel
            nodes={nodes}
            edges={edges}
            particles={particles}
            activeNodes={activeNodes}
            nodeById={nodeById}
          />
          <EventStreamPanel events={events} tps={metrics.tps} />
        </div>

        <MetricsBar
          metrics={metrics}
          sparkline={sparkline}
          errColor={errColor}
          totalFlows={totalFlows}
          platformMetrics={platformMetrics}
        />
      </div>
    </>
  );
}

// ═══════════════════════════════════════════════════════════════════════════
// HEADER
// ═══════════════════════════════════════════════════════════════════════════

function Header({ metrics, errColor, isLive, totalFlows, platformMetrics }) {
  const stats = [
    { label: "TOTAL EVENTS", val: metrics.total.toLocaleString(), color: "#94a3b8" },
    { label: "TPS",          val: metrics.tps,                   color: "#22d3ee" },
    { label: "P50 LATENCY",  val: `${metrics.latency}ms`,        color: "#c084fc" },
    { label: "ERROR RATE",   val: `${metrics.errRate}%`,         color: errColor  },
    { label: "ACTIVE FLOWS", val: `${metrics.flows}/${totalFlows}`, color: "#34d399" },
    { label: "DLQ",          val: metrics.dlq,                   color: metrics.dlq > 8 ? "#ef4444" : "#334155" },
  ];

  // Add platform metrics if available
  if (platformMetrics) {
    stats.push({
      label: "PRODUCTS",
      val: platformMetrics.data_products || 0,
      color: "#22d3ee",
    });
  }

  return (
    <div style={S.header}>
      {/* Wordmark */}
      <div style={{ display: "flex", alignItems: "baseline", gap: 8 }}>
        <span style={S.wordmarkMain}>MESHED</span>
        <span style={S.wordmarkSub}>MONITOR</span>
      </div>

      {/* Live indicator */}
      <div style={{ display: "flex", alignItems: "center", gap: 5 }}>
        <div className="live" style={{
          ...S.liveDot,
          background: isLive ? "#22c55e" : "#fbbf24",
          boxShadow: `0 0 6px ${isLive ? "#22c55e" : "#fbbf24"}`,
        }} />
        <span style={{ fontSize: 9, color: isLive ? "#22c55e" : "#fbbf24", letterSpacing: ".12em" }}>
          {isLive ? "LIVE" : "SIM"}
        </span>
      </div>

      <div style={S.divider} />

      {/* Legend */}
      <div style={{ display: "flex", gap: 10 }}>
        {Object.entries(C).map(([type, { color }]) => (
          <div key={type} style={{ display: "flex", alignItems: "center", gap: 4 }}>
            <div style={{ width: 6, height: 6, borderRadius: "50%", background: color, boxShadow: `0 0 4px ${color}` }} />
            <span style={{ fontSize: 8, color, letterSpacing: ".1em", textTransform: "uppercase" }}>{type}</span>
          </div>
        ))}
      </div>

      <div style={{ flex: 1 }} />

      {/* KPI row */}
      {stats.map((s) => (
        <div key={s.label} style={{ textAlign: "right", minWidth: 70 }}>
          <div style={{ fontSize: 7, color: "#1e3040", letterSpacing: ".14em", marginBottom: 2 }}>{s.label}</div>
          <div style={{ fontSize: 14, fontWeight: 600, color: s.color }}>{s.val}</div>
        </div>
      ))}
    </div>
  );
}

// ═══════════════════════════════════════════════════════════════════════════
// TOPOLOGY PANEL
// ═══════════════════════════════════════════════════════════════════════════

const COLUMNS = [
  { label: "PRODUCERS",  x: 90,  color: C.producer.color  },
  { label: "BROKERS",    x: 295, color: C.broker.color    },
  { label: "PROCESSORS", x: 500, color: C.processor.color },
  { label: "CONSUMERS",  x: 705, color: C.consumer.color  },
];

function TopologyPanel({ nodes, edges, particles, activeNodes, nodeById }) {
  return (
    <div style={{ flex: 1, position: "relative", overflow: "hidden" }}>
      {/* Grid background */}
      <svg style={{ position: "absolute", inset: 0, width: "100%", height: "100%", opacity: 0.07 }}>
        <defs>
          <pattern id="grid" width="36" height="36" patternUnits="userSpaceOnUse">
            <path d="M36,0 L0,0 0,36" fill="none" stroke="#22d3ee" strokeWidth=".5" />
          </pattern>
        </defs>
        <rect width="100%" height="100%" fill="url(#grid)" />
      </svg>

      {/* Main topology SVG */}
      <svg viewBox="0 0 810 500" style={{ width: "100%", height: "100%" }} preserveAspectRatio="xMidYMid meet">
        <defs>
          {Object.entries(C).map(([type, { color }]) => (
            <marker key={type} id={`arrow-${type}`} markerWidth="5" markerHeight="5" refX="4.5" refY="2.5" orient="auto">
              <path d="M0,0 L5,2.5 L0,5 Z" fill={color} opacity=".45" />
            </marker>
          ))}
        </defs>

        {/* Column headers */}
        {COLUMNS.map((col) => (
          <text key={col.label} x={col.x} y={28} textAnchor="middle"
            fontSize={8} letterSpacing=".16em" fontWeight="600"
            fill={col.color} opacity=".35"
            fontFamily="'JetBrains Mono',monospace">{col.label}</text>
        ))}

        {/* Edges */}
        {edges.map((edge) => {
          const f = nodes.find((n) => n.id === edge.from);
          const t = nodes.find((n) => n.id === edge.to);
          if (!f || !t) return null;
          return (
            <path key={edge.id} d={bezier(f, t)} fill="none"
              stroke={C[f.type]?.color || "#475569"} strokeWidth="1"
              strokeOpacity=".16"
              markerEnd={`url(#arrow-${f.type})`} />
          );
        })}

        {/* Animated particles */}
        {particles.map((p) => (
          <circle key={p.id} r={p.isErr ? 4.5 : 3}
            fill={p.isErr ? "#ef4444" : p.color}
            style={{ filter: `drop-shadow(0 0 ${p.isErr ? 9 : 5}px ${p.isErr ? "#ef4444" : p.color})` }}>
            <animateMotion dur={`${p.dur}ms`} begin="0s" repeatCount="1" fill="freeze" path={p.path} />
          </circle>
        ))}

        {/* Nodes */}
        {nodes.map((node) => {
          const { color, dim } = C[node.type] || C.consumer;
          const active = activeNodes.has(node.id);
          const W = 116, H = 46;
          return (
            <g key={node.id} transform={`translate(${node.x},${node.y})`}>
              {active && (
                <rect x={-W / 2 - 6} y={-H / 2 - 6} width={W + 12} height={H + 12} rx={8}
                  fill={color} opacity=".07" style={{ filter: "blur(10px)" }} />
              )}
              <rect x={-W / 2} y={-H / 2} width={W} height={H} rx={3}
                fill={active ? "#060f1a" : "#040a12"}
                stroke={color}
                strokeWidth={active ? 1.5 : 0.5}
                strokeOpacity={active ? 1 : 0.35} />
              <rect x={-W / 2} y={-H / 2} width={3} height={H} rx={1.5}
                fill={color} opacity={active ? 1 : 0.5} />
              <circle cx={W / 2 - 8} cy={-H / 2 + 8} r={3}
                fill={active ? color : dim}
                style={active ? { filter: `drop-shadow(0 0 5px ${color})` } : {}} />
              <text y={-7} textAnchor="middle" fontSize={10} fontWeight="600"
                fill={active ? color : "#475569"}
                fontFamily="'JetBrains Mono',monospace">{node.label}</text>
              <text y={9} textAnchor="middle" fontSize={7.5}
                fill={active ? "#2d4460" : "#1a2d42"}
                fontFamily="'JetBrains Mono',monospace">{node.sub}</text>
            </g>
          );
        })}
      </svg>
    </div>
  );
}

// ═══════════════════════════════════════════════════════════════════════════
// EVENT STREAM PANEL
// ═══════════════════════════════════════════════════════════════════════════

function EventStreamPanel({ events, tps }) {
  return (
    <div style={S.streamPanel}>
      <div style={S.streamHeader}>
        <span style={{ fontSize: 8, letterSpacing: ".16em", color: "#1e3a5f" }}>EVENT STREAM</span>
        <div style={{ display: "flex", alignItems: "center", gap: 5 }}>
          <div className="live" style={{ width: 5, height: 5, borderRadius: "50%", background: "#22d3ee" }} />
          <span style={{ fontSize: 9, color: "#22d3ee" }}>{tps} ev/s</span>
        </div>
      </div>

      <div style={{ flex: 1, overflowY: "auto", padding: "2px 0" }}>
        {events.map((ev, i) => (
          <div key={ev.id} className={i === 0 ? "ev" : ""} style={{
            padding: "5px 14px",
            display: "grid",
            gridTemplateColumns: "90px 1fr",
            gap: 6,
            borderBottom: "1px solid #040a12",
            background: i === 0 ? "#050e1a" : "transparent",
          }}>
            <div style={{ fontSize: 9, color: "#1a2d42", fontVariantNumeric: "tabular-nums", paddingTop: 1 }}>
              {ev.ts}
            </div>
            <div>
              <div style={{ fontSize: 10, fontWeight: 500, marginBottom: 2, color: ev.isErr ? "#ef4444" : "#7a8fa8" }}>
                {ev.isErr && <span style={{ marginRight: 4, color: "#ef4444" }}>✕</span>}
                {ev.type}
              </div>
              <div style={{ fontSize: 9, color: "#1e3040" }}>
                <span style={{ color: C[ev.fromType]?.color || "#475569" }}>{ev.from}</span>
                <span style={{ color: "#0d1e30" }}> → </span>
                <span style={{ color: "#2d4460" }}>{ev.to}</span>
                <span style={{ color: "#0f2236", marginLeft: 6 }}>{ev.lat}ms · {ev.kb}kb</span>
              </div>
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}

// ═══════════════════════════════════════════════════════════════════════════
// METRICS BAR
// ═══════════════════════════════════════════════════════════════════════════

function MetricsBar({ metrics, sparkline, errColor, totalFlows, platformMetrics }) {
  return (
    <div style={S.metricsBar}>
      <MCard label="THROUGHPUT" val={`${metrics.tps} ev/s`} color="#22d3ee">
        <Spark data={sparkline} color="#22d3ee" />
      </MCard>
      <MCard label="P50 LATENCY" val={`${metrics.latency}ms`} color="#c084fc">
        <Bar pct={metrics.latency / 200} color="#c084fc" />
      </MCard>
      <MCard label="ERROR RATE" val={`${metrics.errRate}%`} color={errColor}>
        <span style={{ fontSize: 8, letterSpacing: ".1em", color: metrics.errRate > 2 ? "#ef444466" : "#22c55e66" }}>
          {metrics.errRate > 2 ? "⚠ DEGRADED" : "● NOMINAL"}
        </span>
      </MCard>
      <MCard label="ACTIVE FLOWS" val={`${metrics.flows}/${totalFlows}`} color="#34d399">
        <FlowDots total={totalFlows} active={metrics.flows} />
      </MCard>
      <MCard label="DLQ COUNT" val={metrics.dlq} color={metrics.dlq > 8 ? "#ef4444" : "#2d4460"}>
        <span style={{ fontSize: 8, color: "#1a2d42", letterSpacing: ".1em" }}>DEAD LETTER</span>
      </MCard>
      {platformMetrics ? (
        <MCard label="DATA PRODUCTS" val={platformMetrics.data_products} color="#22d3ee">
          <span style={{ fontSize: 8, color: "#1a2d42", letterSpacing: ".1em" }}>
            {platformMetrics.output_ports} PORTS · {platformMetrics.contracts} SLAs
          </span>
        </MCard>
      ) : (
        <MCard label="SESSION TOTAL" val={metrics.total.toLocaleString()} color="#334155">
          <span style={{ fontSize: 8, color: "#1a2d42", letterSpacing: ".1em" }}>ALL EVENTS</span>
        </MCard>
      )}
    </div>
  );
}

function MCard({ label, val, color, children }) {
  return (
    <div style={{ flex: 1, borderRight: "1px solid #0d1e30", padding: "11px 14px", display: "flex", flexDirection: "column", justifyContent: "space-between" }}>
      <div style={{ fontSize: 7, color: "#1a2d42", letterSpacing: ".16em" }}>{label}</div>
      <div style={{ fontSize: 17, fontWeight: 600, color, lineHeight: 1 }}>{val}</div>
      <div>{children}</div>
    </div>
  );
}

function Spark({ data, color }) {
  const max = Math.max(...data, 1), W = 110, H = 18;
  const pts = data.map((v, i) => `${(i / (data.length - 1)) * W},${H - (v / max) * H}`).join(" ");
  return (
    <svg width={W} height={H}>
      <polyline points={pts} fill="none" stroke={color} strokeWidth="1.5" opacity=".6" />
    </svg>
  );
}

function Bar({ pct, color }) {
  return (
    <div style={{ height: 3, background: "#0d1e30", borderRadius: 2, overflow: "hidden" }}>
      <div style={{ height: "100%", width: `${Math.min(pct, 1) * 100}%`, background: color, borderRadius: 2, transition: "width .7s ease" }} />
    </div>
  );
}

function FlowDots({ total, active }) {
  return (
    <div style={{ display: "flex", gap: 3, flexWrap: "wrap" }}>
      {Array.from({ length: Math.min(total, 20) }).map((_, i) => (
        <div key={i} style={{
          width: 6, height: 6, borderRadius: "50%",
          background: i < active ? "#34d399" : "#0d1e30",
          boxShadow: i < active ? "0 0 4px #34d399" : "none",
          transition: "all .5s",
        }} />
      ))}
    </div>
  );
}

// ═══════════════════════════════════════════════════════════════════════════
// STYLES
// ═══════════════════════════════════════════════════════════════════════════

const GLOBAL_CSS = `
  @import url('https://fonts.googleapis.com/css2?family=JetBrains+Mono:wght@300;400;500;600&family=Orbitron:wght@700;900&display=swap');
  *, *::before, *::after { box-sizing: border-box; margin: 0; padding: 0; }
  body { background: #020408; overflow: hidden; }
  ::-webkit-scrollbar { width: 3px; }
  ::-webkit-scrollbar-thumb { background: #0f2236; border-radius: 2px; }
  @keyframes blink { 0%,100% { opacity:1 } 50% { opacity:.15 } }
  @keyframes slideIn { from { opacity:0; transform:translateY(-5px) } to { opacity:1; transform:none } }
  .live { animation: blink 1.8s infinite; }
  .ev   { animation: slideIn .15s ease-out; }
`;

const S = {
  root: {
    display: "flex", flexDirection: "column", height: "100%",
    background: "#020408", fontFamily: "'JetBrains Mono',monospace",
    color: "#94a3b8", fontSize: 12, overflow: "hidden",
  },
  scanline: {
    display: "none",
  },
  header: {
    height: 52, flexShrink: 0, display: "flex", alignItems: "center",
    padding: "0 20px", gap: 20,
    background: "linear-gradient(180deg,#04080f,#020408)",
    borderBottom: "1px solid #0d1e30", position: "relative", zIndex: 10,
  },
  wordmarkMain: {
    fontFamily: "'Orbitron',sans-serif", fontSize: 15, fontWeight: 900,
    color: "#22d3ee", letterSpacing: ".1em",
  },
  wordmarkSub: {
    fontFamily: "'Orbitron',sans-serif", fontSize: 9, fontWeight: 700,
    color: "#1e3a5f", letterSpacing: ".18em",
  },
  liveDot: {
    width: 7, height: 7, borderRadius: "50%",
    background: "#22c55e", boxShadow: "0 0 6px #22c55e",
  },
  divider: { width: 1, height: 20, background: "#0d1e30", margin: "0 4px" },
  body: { display: "flex", flex: 1, overflow: "hidden", position: "relative", zIndex: 1 },
  streamPanel: {
    width: 340, flexShrink: 0,
    borderLeft: "1px solid #0d1e30",
    display: "flex", flexDirection: "column", overflow: "hidden",
  },
  streamHeader: {
    padding: "10px 14px", flexShrink: 0,
    borderBottom: "1px solid #0d1e30",
    display: "flex", alignItems: "center", justifyContent: "space-between",
  },
  metricsBar: {
    height: 82, flexShrink: 0,
    borderTop: "1px solid #0d1e30",
    background: "#020408", display: "flex",
    position: "relative", zIndex: 10,
  },
};
