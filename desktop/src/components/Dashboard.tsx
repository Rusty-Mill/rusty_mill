import { createSignal, For, onMount, Show } from "solid-js";
import { store } from "../store";
import { ipc } from "../ipc";

/// Harness dashboard (Cmd/Ctrl+Shift+H): verification stream, evidence journal,
/// entropy, M-HIR, token budget. All data via `invoke`; refreshes on open.
export default function Dashboard(props: { onClose: () => void }) {
  const [evidence, setEvidence] = createSignal<unknown[]>([]);
  const [cumulative, setCumulative] = createSignal(0);

  onMount(async () => {
    try {
      setEvidence(await ipc.evidenceRecent(30));
    } catch (_) {
      /* */
    }
    try {
      const hist = await ipc.entropyHistory();
      setCumulative(hist.cumulative_delta ?? 0);
    } catch (_) {
      /* */
    }
  });

  return (
    <div class="absolute inset-0 z-20 overflow-y-auto bg-rk-bg/95 p-6">
      <div class="mx-auto max-w-5xl">
        <div class="mb-4 flex items-center justify-between">
          <h2 class="text-lg font-semibold">Harness Dashboard</h2>
          <button class="rounded px-3 py-1 text-sm text-rk-muted hover:bg-rk-panel-2" onClick={props.onClose}>
            Close (Esc)
          </button>
        </div>

        <div class="grid grid-cols-1 gap-4 md:grid-cols-2">
          <section class="rounded border border-rk-border bg-rk-panel p-3">
            <h3 class="mb-2 text-sm font-semibold text-rk-gold">M-HIR</h3>
            <Show when={store.mhir()} fallback={<p class="text-xs text-rk-muted">(no data)</p>}>
              {(m) => (
                <div class="mono text-xs text-rk-text">
                  <div>
                    {m().n_interventions} avoidable / {m().n_turns} turns ={" "}
                    {(m().rate * 100).toFixed(1)}%
                  </div>
                  <div class="mt-1 text-rk-muted">
                    excluded: {m().n_unavoidable} unavoidable, {m().n_benign} benign
                  </div>
                  <For each={Object.entries(m().breakdown ?? {})}>
                    {([k, v]) => (
                      <div class="mt-1">
                        {k} <span class="text-rk-muted">{v}</span>
                      </div>
                    )}
                  </For>
                </div>
              )}
            </Show>
          </section>

          <section class="rounded border border-rk-border bg-rk-panel p-3">
            <h3 class="mb-2 text-sm font-semibold text-rk-gold">Token Budget</h3>
            <Show when={store.budget()} fallback={<p class="text-xs text-rk-muted">(no data)</p>}>
              {(b) => (
                <div class="mono text-xs text-rk-text">
                  <div>
                    {b().used.toLocaleString()} / {b().limit.toLocaleString()} (
                    {(b().fraction * 100).toFixed(0)}%)
                  </div>
                  <div class="mt-1 text-rk-muted">
                    session total {b().session_total.toLocaleString()} · compactions{" "}
                    {b().compactions}
                  </div>
                </div>
              )}
            </Show>
          </section>

          <section class="rounded border border-rk-border bg-rk-panel p-3">
            <h3 class="mb-2 text-sm font-semibold text-rk-gold">Verification Stream</h3>
            <div class="max-h-48 overflow-y-auto">
              <Show
                when={store.turns.length}
                fallback={<p class="text-xs text-rk-muted">(no turns yet)</p>}
              >
                <For each={[...store.turns].reverse()}>
                  {(t) => (
                    <div class="flex items-center gap-2 py-0.5 mono text-xs">
                      <span class={t.verified ? "text-rk-green" : "text-rk-red"}>
                        {t.verified ? "✓" : "✗"}
                      </span>
                      <span class="truncate text-rk-text">{t.user_message}</span>
                    </div>
                  )}
                </For>
              </Show>
            </div>
          </section>

          <section class="rounded border border-rk-border bg-rk-panel p-3">
            <h3 class="mb-2 text-sm font-semibold text-rk-gold">
              Entropy <span class="text-rk-muted">(cumulative Δ {cumulative()})</span>
            </h3>
            <div class="max-h-48 overflow-y-auto">
              <Show
                when={store.entropy.length}
                fallback={<p class="text-xs text-rk-muted">(no findings)</p>}
              >
                <For each={[...store.entropy].reverse()}>
                  {(a) => (
                    <div class="py-0.5 mono text-xs">
                      <span class={(a.delta ?? 0) > 0 ? "text-rk-red" : "text-rk-green"}>
                        Δ{a.delta ?? 0}
                      </span>{" "}
                      <span class="text-rk-muted">{a.findings?.length ?? 0} findings</span>
                    </div>
                  )}
                </For>
              </Show>
            </div>
          </section>

          <section class="rounded border border-rk-border bg-rk-panel p-3 md:col-span-2">
            <h3 class="mb-2 text-sm font-semibold text-rk-gold">Evidence Journal</h3>
            <div class="max-h-60 overflow-y-auto">
              <Show
                when={evidence().length}
                fallback={<p class="text-xs text-rk-muted">(no evidence yet)</p>}
              >
                <For each={evidence()}>
                  {(e) => (
                    <pre class="border-b border-rk-border/50 py-1 mono text-xs text-rk-muted">
                      {JSON.stringify(e)}
                    </pre>
                  )}
                </For>
              </Show>
            </div>
          </section>
        </div>
      </div>
    </div>
  );
}
