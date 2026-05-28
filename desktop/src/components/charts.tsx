import { For, Show } from "solid-js";

/// A plain-SVG donut for the token budget (used / limit). Colour shifts at the
/// 80% / 90% thresholds, mirroring the header.
export function TokenDonut(props: { used: number; limit: number; size?: number }) {
  const size = () => props.size ?? 120;
  const r = () => size() / 2 - 10;
  const c = () => 2 * Math.PI * r();
  const frac = () => (props.limit > 0 ? Math.min(1, props.used / props.limit) : 0);
  const color = () => (frac() >= 0.9 ? "#f85149" : frac() >= 0.8 ? "#d29922" : "#3fb950");
  return (
    <svg width={size()} height={size()} viewBox={`0 0 ${size()} ${size()}`}>
      <circle
        cx={size() / 2}
        cy={size() / 2}
        r={r()}
        fill="none"
        stroke="#2a2f3a"
        stroke-width="10"
      />
      <circle
        cx={size() / 2}
        cy={size() / 2}
        r={r()}
        fill="none"
        stroke={color()}
        stroke-width="10"
        stroke-linecap="round"
        stroke-dasharray={`${frac() * c()} ${c()}`}
        transform={`rotate(-90 ${size() / 2} ${size() / 2})`}
      />
      <text
        x="50%"
        y="50%"
        text-anchor="middle"
        dominant-baseline="middle"
        fill="#d7dce5"
        font-size="16"
        class="mono"
      >
        {Math.round(frac() * 100)}%
      </text>
    </svg>
  );
}

/// Per-turn entropy delta bars (green = neutral/negative, red = burden).
export function EntropyBars(props: { values: number[]; height?: number }) {
  const h = () => props.height ?? 80;
  const max = () => Math.max(1, ...props.values.map((v) => Math.abs(v)));
  return (
    <Show
      when={props.values.length > 0}
      fallback={<p class="text-xs text-rk-muted">(no entropy findings)</p>}
    >
      <div class="flex items-end gap-0.5" style={{ height: `${h()}px` }}>
        <For each={props.values}>
          {(v) => (
            <div
              class="w-2 rounded-t"
              classList={{ "bg-rk-red": v > 0, "bg-rk-green": v <= 0 }}
              style={{ height: `${(Math.abs(v) / max()) * h()}px` }}
              title={`Δ${v}`}
            />
          )}
        </For>
      </div>
    </Show>
  );
}

/// Horizontal bars for the M-HIR intervention breakdown.
export function HBars(props: { data: [string, number][] }) {
  const max = () => Math.max(1, ...props.data.map(([, v]) => v));
  return (
    <Show
      when={props.data.length > 0}
      fallback={<p class="text-xs text-rk-muted">(no interventions)</p>}
    >
      <div class="flex flex-col gap-1">
        <For each={props.data}>
          {([label, value]) => (
            <div class="flex items-center gap-2 mono text-xs">
              <span class="w-40 truncate text-rk-muted">{label}</span>
              <div class="h-3 flex-1 rounded bg-rk-bg">
                <div
                  class="h-3 rounded bg-rk-gold"
                  style={{ width: `${(value / max()) * 100}%` }}
                />
              </div>
              <span class="w-6 text-right text-rk-text">{value}</span>
            </div>
          )}
        </For>
      </div>
    </Show>
  );
}
