/**
 * reverseTrace.js
 *
 * Pure JavaScript port of `rusty-meshed-trace` (rusty_mill →
 * crates/rusty_meshed/crates/rusty-meshed-trace): the domain maturity
 * ladder and the reverse-trace algorithm (outcome → domains → sources).
 *
 * The Rust crate is the reference implementation; this file exists so the
 * dashboard can run the trace client-side in v1 (fully simulated, no
 * backend). It consumes exactly the scenario JSON the crate emits
 * (`Scenario::to_json`) and produces exactly the report JSON it emits
 * (`TraceReport::to_json`) -- `scripts/trace-parity.mjs` checks the two
 * agree on the bundled scenario. When a live feed lands (P2), the view can
 * swap `traceOutcome` for a fetch of the crate's output without changing
 * shape.
 *
 * No React in here: everything is a plain function of its inputs.
 */

// ── Maturity ladder ───────────────────────────────────────────────────────

export const MATURITY = [
  { level: 0, name: "Tribal",     color: "#ef4444", blurb: "In people's heads or undocumented process. No artifact." },
  { level: 1, name: "Documented", color: "#f97316", blurb: "Static artifacts: slides, PDFs, a personal spreadsheet." },
  { level: 2, name: "Structured", color: "#fbbf24", blurb: "In a system with a schema, but siloed; extraction is manual." },
  { level: 3, name: "Accessible", color: "#84cc16", blurb: "Programmatically queryable: API, DB, scheduled export." },
  { level: 4, name: "Integrated", color: "#22c55e", blurb: "On the mesh: governed, contracted, flows automatically." },
];

export const maturityLabel = (level) => `L${level} ${MATURITY[level]?.name ?? "?"}`;
export const maturityColor = (level) => MATURITY[level]?.color ?? "#475569";

// ── Edge states ───────────────────────────────────────────────────────────

export const EDGE_STATE = {
  satisfied: { label: "Satisfied", color: "#22c55e", dash: null,    blurb: "Domain meets the floor. Data flows." },
  blocked:   { label: "Blocked",   color: "#ef4444", dash: "6 4",   blurb: "Blocking gap. Nothing gets through." },
  degraded:  { label: "Degraded",  color: "#fbbf24", dash: null,    blurb: "Degrading gap. Partial, throttled flow." },
  missing:   { label: "Missing",   color: "#475569", dash: "1.5 4", blurb: "Optional and unmet. Panel would be blank." },
};

const WEIGHT = { blocking: 3.0, degrading: 2.0, optional: 1.0 };
const CRIT_RANK = { blocking: 2, degrading: 1, optional: 0 };

// ── Lookups ───────────────────────────────────────────────────────────────

export const domainById  = (scenario, id) => scenario.domains.find((d) => d.id === id);
export const sourceById  = (scenario, id) => scenario.sources.find((s) => s.id === id);
export const outcomeById = (scenario, id) => scenario.outcomes.find((o) => o.id === id);
export const sourcesOf   = (scenario, domainId) => scenario.sources.filter((s) => s.domain === domainId);

/** What-if: a copy of `scenario` with one domain's maturity changed. */
export function withMaturity(scenario, domainId, level) {
  return {
    ...scenario,
    domains: scenario.domains.map((d) => (d.id === domainId ? { ...d, maturity: level } : d)),
  };
}

// ── The reverse trace (mirrors rusty_meshed_trace::trace) ─────────────────

/**
 * Traces one outcome back through the domains it requires and the sources
 * behind them. Returns the same shape as `TraceReport::to_json()`.
 *
 * Per requirement: gap = max(0, min_maturity - domain.maturity); gap == 0
 * is "satisfied", else the criticality decides blocked / degraded /
 * missing. Any blocked → not_achievable; else any degraded →
 * partial(satisfied_weight / total_weight); else full. Bottlenecks come
 * back worst-first (criticality, then gap, then domain id).
 */
export function traceOutcome(scenario, outcomeId) {
  const outcome = outcomeById(scenario, outcomeId);
  if (!outcome) throw new Error(`unknown outcome ${JSON.stringify(outcomeId)}`);

  const edges = [];
  const sourceEdges = [];
  const bottlenecks = [];
  let anyBlocked = false, anyDegraded = false;
  let satisfiedWeight = 0, totalWeight = 0;

  for (const req of outcome.requires) {
    const domain = domainById(scenario, req.domain);
    if (!domain) throw new Error(`outcome ${JSON.stringify(outcomeId)} requires unknown domain ${JSON.stringify(req.domain)}`);
    const gap = Math.max(0, req.min_maturity - domain.maturity);
    const state = gap === 0
      ? "satisfied"
      : { blocking: "blocked", degrading: "degraded", optional: "missing" }[req.criticality];

    const weight = WEIGHT[req.criticality];
    totalWeight += weight;
    if (state === "satisfied") satisfiedWeight += weight;
    else if (state === "blocked") anyBlocked = true;
    else if (state === "degraded") anyDegraded = true;

    edges.push({ from: { kind: "domain", id: domain.id }, to: { kind: "outcome", id: outcome.id }, state });
    const sources = sourcesOf(scenario, domain.id);
    for (const s of sources) {
      sourceEdges.push({ from: { kind: "source", id: s.id }, to: { kind: "domain", id: domain.id }, state });
    }

    if (state !== "satisfied") {
      bottlenecks.push({
        domain: domain.id,
        domain_name: domain.name,
        owner: domain.owner,
        current: domain.maturity,
        required: req.min_maturity,
        gap,
        criticality: req.criticality,
        state,
        sources: sources.map((s) => s.id),
      });
    }
  }
  edges.push(...sourceEdges);

  let achievable;
  if (anyBlocked) achievable = { kind: "not_achievable" };
  else if (anyDegraded) {
    // Math.fround: the Rust side computes this in f32.
    const fraction = totalWeight > 0 ? Math.fround(Math.fround(satisfiedWeight) / Math.fround(totalWeight)) : 1;
    achievable = { kind: "partial", fraction };
  } else achievable = { kind: "full" };

  bottlenecks.sort((a, b) =>
    (CRIT_RANK[b.criticality] - CRIT_RANK[a.criticality])
    || (b.gap - a.gap)
    || (a.domain < b.domain ? -1 : a.domain > b.domain ? 1 : 0));

  return { outcome: outcome.id, outcome_name: outcome.name, achievable, bottlenecks, edges };
}

export const traceAll = (scenario) => scenario.outcomes.map((o) => traceOutcome(scenario, o.id));

/** State of the edge from `domainId` to the traced outcome, or undefined. */
export function domainState(report, domainId) {
  return report.edges.find((e) => e.from.kind === "domain" && e.from.id === domainId && e.to.kind === "outcome")?.state;
}

/** 0..1 for the gauge; the verdict text for the label. */
export function fidelitySummary(achievable) {
  switch (achievable.kind) {
    case "full":           return { fraction: 1, label: "FULL", color: EDGE_STATE.satisfied.color };
    case "partial":        return { fraction: achievable.fraction, label: `PARTIAL · ${Math.round(achievable.fraction * 100)}%`, color: EDGE_STATE.degraded.color };
    default:               return { fraction: 0, label: "NOT ACHIEVABLE", color: EDGE_STATE.blocked.color };
  }
}

// ── Markdown gap summary (mirrors TraceReport::to_markdown) ───────────────

const escapeCell = (s) => String(s).replace(/\|/g, "\\|");

export function toMarkdown(report, scenario) {
  const lines = [];
  lines.push(`# Gap summary: ${report.outcome_name}`, "");
  const outcome = outcomeById(scenario, report.outcome);
  if (outcome?.description?.trim()) lines.push(`> ${outcome.description.trim()}`, "");
  const verdict = report.achievable.kind === "full" ? "Full"
    : report.achievable.kind === "partial" ? `Partial (${Math.round(report.achievable.fraction * 100)}%)`
    : "Not achievable";
  lines.push(`**Achievable today:** ${verdict}`, "");

  if (report.bottlenecks.length === 0) {
    lines.push("Every required domain meets its maturity floor. No gaps to fund.", "");
  } else {
    lines.push("## What to fund first", "");
    lines.push("| # | Domain | Owner | Today | Needed | Gap | Impact | Data lives in |");
    lines.push("|---|---|---|---|---|---|---|---|");
    report.bottlenecks.forEach((b, i) => {
      const impact = { blocked: "**Blocking**", degraded: "Degrading", missing: "Optional" }[b.state] ?? "-";
      const sources = b.sources.length === 0 ? "_none recorded_"
        : b.sources.map((id) => {
            const s = sourceById(scenario, id);
            return s ? `${escapeCell(s.name)} (${s.kind})` : escapeCell(id);
          }).join(", ");
      lines.push(`| ${i + 1} | ${escapeCell(b.domain_name)} | ${escapeCell(b.owner)} | ${maturityLabel(b.current)} | ${maturityLabel(b.required)} | +${b.gap} | ${impact} | ${sources} |`);
    });
    lines.push("");
  }

  const satisfied = report.edges
    .filter((e) => e.state === "satisfied" && e.from.kind === "domain" && e.to.kind === "outcome")
    .map((e) => domainById(scenario, e.from.id))
    .filter(Boolean);
  if (satisfied.length) {
    lines.push("## Already in place", "");
    for (const d of satisfied) lines.push(`- ${d.name} -- ${maturityLabel(d.maturity)} (${d.owner})`);
    lines.push("");
  }

  lines.push("---", "", `_Maturity ladder: ${MATURITY.map((m) => maturityLabel(m.level)).join(" · ")}._`);
  return lines.join("\n") + "\n";
}
