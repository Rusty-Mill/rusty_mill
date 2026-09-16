/**
 * App.jsx
 *
 * Top-level shell: a slim tab bar switching between the three views of the
 * simulator — "Mesh Ops" (DataMeshMonitor, live traffic + topology),
 * "Transformation" (TransformationSimulator, the legacy-to-mesh migration
 * journey) and "Reverse Trace" (ReverseTrace, outcome → domains → sources
 * with domain maturity). Kept as a thin wrapper so each view stays a
 * self-contained, independently-styled full-height component.
 */

import { useState } from "react";
import DataMeshMonitor from "./DataMeshMonitor.jsx";
import TransformationSimulator from "./TransformationSimulator.jsx";
import ReverseTrace from "./ReverseTrace.jsx";

const TABS = [
  { id: "mesh", label: "MESH OPS" },
  { id: "transform", label: "TRANSFORMATION" },
  { id: "trace", label: "REVERSE TRACE" },
];

export default function App() {
  // Deep-linkable: /#trace opens the Reverse Trace view directly, which
  // is what a briefing deck links to.
  const [tab, setTab] = useState(
    () => TABS.find((t) => `#${t.id}` === window.location.hash)?.id ?? "mesh",
  );

  return (
    <div style={S.root}>
      <style>{SHARED_CSS}</style>
      <div style={S.tabBar}>
        <span style={S.wordmark}>MESHED</span>
        <div style={{ flex: 1 }} />
        {TABS.map((t) => (
          <button
            key={t.id}
            onClick={() => { setTab(t.id); window.location.hash = t.id; }}
            style={{ ...S.tabButton, ...(tab === t.id ? S.tabButtonActive : {}) }}
          >
            {t.label}
          </button>
        ))}
      </div>
      <div style={S.content}>
        {tab === "mesh" ? <DataMeshMonitor /> : tab === "transform" ? <TransformationSimulator /> : <ReverseTrace />}
      </div>
    </div>
  );
}

// Shared reset + font import + the .live/.ev animation classes both tabs use.
// Kept here (not per-tab) so switching tabs doesn't unmount the styles the
// other tab's components rely on. DataMeshMonitor also injects its own copy
// so it keeps working if ever rendered standalone — harmless duplication.
const SHARED_CSS = `
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
    display: "flex", flexDirection: "column", height: "100%", width: "100%",
    background: "#020408",
  },
  tabBar: {
    height: 28, flexShrink: 0, display: "flex", alignItems: "center", gap: 2,
    padding: "0 12px", background: "#04080f", borderBottom: "1px solid #0d1e30",
    fontFamily: "'JetBrains Mono',monospace", position: "relative", zIndex: 20,
  },
  wordmark: {
    fontFamily: "'Orbitron',sans-serif", fontSize: 10, fontWeight: 900,
    color: "#22d3ee", letterSpacing: ".1em", marginRight: 12,
  },
  tabButton: {
    background: "transparent", border: "none", borderBottom: "2px solid transparent",
    cursor: "pointer", color: "#3d5878", fontSize: 9.5, letterSpacing: ".1em",
    padding: "7px 12px 5px", fontFamily: "'JetBrains Mono',monospace",
  },
  tabButtonActive: { color: "#22d3ee", borderBottom: "2px solid #22d3ee" },
  content: { flex: 1, overflow: "hidden", position: "relative" },
};
