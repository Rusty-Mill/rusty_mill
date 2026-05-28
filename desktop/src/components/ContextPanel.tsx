import { createSignal, For, Show, createEffect } from "solid-js";
import { store, type ContextTab } from "../store";
import { ipc } from "../ipc";
import type { MemoryEntry } from "../contract";

const TABS: { id: ContextTab; label: string }[] = [
  { id: "terminal", label: "Terminal" },
  { id: "editor", label: "Editor" },
  { id: "git", label: "Git" },
  { id: "memory", label: "Memory" },
  { id: "web", label: "Web" },
];

/// Secondary panel: auto-focuses a tab based on the agent's tool calls (unless
/// the human pinned a tab). v1 renders plain views; xterm.js + CodeMirror 6 land
/// in the context-panel pass.
export default function ContextPanel() {
  const [memories, setMemories] = createSignal<MemoryEntry[]>([]);

  // Lazy-load the memory snapshot when the Memory tab is focused.
  createEffect(() => {
    if (store.activeTab() === "memory") {
      ipc
        .memorySnapshot()
        .then((snap) => setMemories(snap.recent ?? []))
        .catch(() => setMemories([]));
    }
  });

  const lastEdit = () =>
    [...store.turns]
      .reverse()
      .flatMap((t) => t.tool_events)
      .find((e) => e.name === "edit_file" || e.name === "write_file");

  return (
    <div class="flex h-full flex-col bg-rk-panel">
      <div class="flex shrink-0 items-center border-b border-rk-border">
        <For each={TABS}>
          {(tab) => (
            <button
              class="border-b-2 px-3 py-2 text-xs"
              classList={{
                "border-rk-gold text-rk-text": store.activeTab() === tab.id,
                "border-transparent text-rk-muted hover:text-rk-text":
                  store.activeTab() !== tab.id,
              }}
              onClick={() => store.setActiveTab(tab.id)}
            >
              {tab.label}
            </button>
          )}
        </For>
        <div class="flex-1" />
        <button
          class="mr-2 rounded px-2 py-1 text-xs"
          classList={{
            "bg-rk-gold/20 text-rk-gold": store.pinned(),
            "text-rk-muted hover:text-rk-text": !store.pinned(),
          }}
          title="Pin tab (stop auto-switching)"
          onClick={() => store.setPinned(!store.pinned())}
        >
          {store.pinned() ? "📌 Pinned" : "Pin"}
        </button>
      </div>

      <div class="min-h-0 flex-1 overflow-y-auto p-3">
        <Show when={store.activeTab() === "terminal"}>
          <pre class="whitespace-pre-wrap mono text-xs text-rk-text">
            {store.terminalChunks.length
              ? store.terminalChunks.join("")
              : "(no terminal output yet — bash tool output streams here)"}
          </pre>
        </Show>

        <Show when={store.activeTab() === "editor"}>
          <Show
            when={lastEdit()}
            fallback={<p class="text-xs text-rk-muted">(no file edits yet)</p>}
          >
            {(edit) => (
              <div>
                <div class="mb-2 mono text-xs text-rk-blue">{edit().name}</div>
                <pre class="whitespace-pre-wrap mono text-xs text-rk-text">
                  {JSON.stringify(edit().args, null, 2)}
                </pre>
              </div>
            )}
          </Show>
        </Show>

        <Show when={store.activeTab() === "git"}>
          <p class="text-xs text-rk-muted">
            Git diff view lands in the context-panel pass (CodeMirror 6).
          </p>
        </Show>

        <Show when={store.activeTab() === "memory"}>
          <Show
            when={memories().length > 0}
            fallback={<p class="text-xs text-rk-muted">(no memories yet)</p>}
          >
            <For each={memories()}>
              {(m) => (
                <div class="mb-2 rounded border border-rk-border bg-rk-panel-2 px-2 py-1">
                  <div class="flex items-center gap-2">
                    <span class="mono text-xs text-rk-gold">{m.mem_type ?? "mem"}</span>
                    <Show when={m.validated}>
                      <span class="text-xs text-rk-green">✓</span>
                    </Show>
                    <span class="text-xs text-rk-text">{m.title}</span>
                  </div>
                  <div class="mt-0.5 text-xs text-rk-muted">{m.body}</div>
                </div>
              )}
            </For>
          </Show>
        </Show>

        <Show when={store.activeTab() === "web"}>
          <p class="text-xs text-rk-muted">
            Web fetch/search results render here when the agent browses.
          </p>
        </Show>
      </div>
    </div>
  );
}
