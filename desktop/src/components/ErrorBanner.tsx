import { Show } from "solid-js";
import { store } from "../store";

/// Uniform render of a boundary-error `invoke` rejection (PRD 08). Dismissible;
/// a failed turn already unlocked the composer in `sendTurn`'s catch.
export default function ErrorBanner() {
  return (
    <Show when={store.error()}>
      {(err) => (
        <div class="flex shrink-0 items-center gap-2 border-t border-rk-red/50 bg-rk-red/10 px-4 py-2 text-sm">
          <span class="rounded bg-rk-red/20 px-1.5 py-0.5 mono text-xs uppercase text-rk-red">
            {err().kind}
          </span>
          <span class="flex-1 truncate text-rk-text">{err().message}</span>
          <button
            class="text-rk-muted hover:text-rk-text"
            onClick={() => store.setError(null)}
          >
            ×
          </button>
        </div>
      )}
    </Show>
  );
}
