import { Show } from "solid-js";
import { store } from "../store";

function pct(frac: number): string {
  return `${Math.round(frac * 100)}%`;
}

function budgetColor(frac: number): string {
  if (frac >= 0.9) return "text-rk-red";
  if (frac >= 0.8) return "text-rk-yellow";
  return "text-rk-muted";
}

/// Persistent header: model/mode, token budget, M-HIR. Updated after each turn.
export default function Header(props: {
  onOpenDashboard: () => void;
  onOpenSettings: () => void;
}) {
  return (
    <header class="flex shrink-0 items-center gap-4 border-b border-rk-border bg-rk-panel px-4 py-2">
      <div class="flex items-center gap-2">
        <span class="inline-block h-3 w-3 rounded-sm bg-rk-gold" />
        <span class="font-semibold tracking-wide">Rusty Keys</span>
      </div>

      <Show when={store.config()}>
        {(cfg) => (
          <span class="rounded bg-rk-panel-2 px-2 py-0.5 text-xs uppercase text-rk-muted">
            {cfg().permission_mode}
          </span>
        )}
      </Show>

      <div class="flex-1" />

      <Show when={store.budget()}>
        {(b) => (
          <span class={`mono text-xs ${budgetColor(b().fraction)}`}>
            {b().used.toLocaleString()} / {b().limit.toLocaleString()} ({pct(b().fraction)})
          </span>
        )}
      </Show>

      <Show when={store.mhir()}>
        {(m) => (
          <span class="mono text-xs text-rk-muted">M-HIR {pct(m().rate)}</span>
        )}
      </Show>

      <button
        class="rounded px-2 py-1 text-xs text-rk-text hover:bg-rk-panel-2"
        title="Harness dashboard (Cmd/Ctrl+Shift+H)"
        onClick={props.onOpenDashboard}
      >
        Dashboard
      </button>
      <button
        class="rounded px-2 py-1 text-xs text-rk-text hover:bg-rk-panel-2"
        title="Settings"
        onClick={props.onOpenSettings}
      >
        Settings
      </button>
    </header>
  );
}
