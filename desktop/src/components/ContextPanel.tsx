import { createSignal, For, Show, createEffect } from "solid-js";
import { store, type ContextTab } from "../store";
import { ipc } from "../ipc";
import type { GitStatus, MemoryEntry } from "../contract";
import TerminalView from "./Terminal";
import Editor from "./Editor";
import DiffView from "./DiffView";

const TABS: { id: ContextTab; label: string }[] = [
  { id: "terminal", label: "Terminal" },
  { id: "editor", label: "Editor" },
  { id: "git", label: "Git" },
  { id: "memory", label: "Memory" },
  { id: "web", label: "Web" },
];

interface EditArgs {
  path?: string;
  content?: string;
  oldString?: string;
  newString?: string;
}

/// Secondary panel: auto-focuses a tab based on the agent's tool calls (unless
/// the human pinned a tab). Terminal is xterm.js over `rk://bash_output`; the
/// editor is a CodeMirror 6 diff (edit_file) / read-only view (write_file).
export default function ContextPanel() {
  const [memories, setMemories] = createSignal<MemoryEntry[]>([]);
  const [git, setGit] = createSignal<GitStatus | null>(null);
  const [gitDiff, setGitDiff] = createSignal("");
  const [gitPath, setGitPath] = createSignal<string | null>(null);
  const [gitStaged, setGitStaged] = createSignal(false);

  // Lazy-load the memory snapshot when the Memory tab is focused.
  createEffect(() => {
    if (store.activeTab() === "memory") {
      ipc
        .memorySnapshot()
        .then((snap) => setMemories(snap.recent ?? []))
        .catch(() => setMemories([]));
    }
  });

  // Refresh git status when the Git tab is focused or a new turn settles.
  createEffect(() => {
    void store.turns.length;
    if (store.activeTab() === "git") {
      ipc
        .gitStatus()
        .then((s) => setGit(s))
        .catch(() => setGit({ repo: false }));
    }
  });

  // (Re)load the diff when the selected path / staged scope changes.
  const loadDiff = (path: string | null, staged: boolean) => {
    setGitPath(path);
    setGitStaged(staged);
    ipc
      .gitDiff(path ?? undefined, staged)
      .then((d) => setGitDiff(d.diff))
      .catch(() => setGitDiff(""));
  };

  const lastEdit = () =>
    [...store.turns]
      .reverse()
      .flatMap((t) => t.tool_events)
      .find((e) => e.name === "edit_file" || e.name === "write_file");

  const editArgs = (): EditArgs | null => {
    const edit = lastEdit();
    if (!edit) return null;
    const a = (edit.args ?? {}) as Record<string, unknown>;
    return {
      path: typeof a.path === "string" ? a.path : undefined,
      content: typeof a.content === "string" ? a.content : undefined,
      oldString: typeof a.old_string === "string" ? a.old_string : undefined,
      newString: typeof a.new_string === "string" ? a.new_string : undefined,
    };
  };

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

      <div class="relative min-h-0 flex-1">
        {/* Terminal + editor fill the panel (own their scroll); the rest scroll. */}
        <Show when={store.activeTab() === "terminal"}>
          <div class="absolute inset-0 p-2">
            <TerminalView />
          </div>
        </Show>

        <Show when={store.activeTab() === "editor"}>
          <div class="absolute inset-0 flex flex-col">
            <Show
              when={editArgs()}
              fallback={<p class="p-3 text-xs text-rk-muted">(no file edits yet)</p>}
            >
              {(args) => (
                <>
                  <div class="shrink-0 border-b border-rk-border px-3 py-1 mono text-xs text-rk-blue">
                    {lastEdit()?.name} · {args().path ?? "(unknown path)"}
                  </div>
                  <div class="min-h-0 flex-1">
                    <Editor
                      path={args().path}
                      content={args().content}
                      oldString={args().oldString}
                      newString={args().newString}
                    />
                  </div>
                </>
              )}
            </Show>
          </div>
        </Show>

        <Show when={store.activeTab() === "git"}>
          <div class="absolute inset-0 flex flex-col">
            <Show
              when={git()?.repo}
              fallback={
                <p class="p-3 text-xs text-rk-muted">(not a git repository)</p>
              }
            >
              {(() => {
                const g = () => git() as GitStatus;
                return (
                  <>
                    <div class="flex shrink-0 items-center gap-3 border-b border-rk-border px-3 py-1.5 mono text-xs">
                      <span class="text-rk-gold">{g().branch}</span>
                      <Show when={(g().ahead ?? 0) > 0}>
                        <span class="text-rk-green">↑{g().ahead}</span>
                      </Show>
                      <Show when={(g().behind ?? 0) > 0}>
                        <span class="text-rk-red">↓{g().behind}</span>
                      </Show>
                      <div class="flex-1" />
                      <label class="flex items-center gap-1 text-rk-muted">
                        <input
                          type="checkbox"
                          checked={gitStaged()}
                          onChange={(e) => loadDiff(gitPath(), e.currentTarget.checked)}
                        />
                        staged
                      </label>
                    </div>
                    <div class="flex min-h-0 flex-1">
                      <div class="w-56 shrink-0 overflow-y-auto border-r border-rk-border p-1">
                        <Show
                          when={(g().files ?? []).length > 0}
                          fallback={<p class="p-2 text-xs text-rk-muted">(clean tree)</p>}
                        >
                          <For each={g().files}>
                            {(f) => (
                              <button
                                class="block w-full truncate rounded px-2 py-0.5 text-left mono text-xs hover:bg-rk-panel-2"
                                classList={{ "bg-rk-panel-2": gitPath() === f.path }}
                                onClick={() => loadDiff(f.path, gitStaged())}
                              >
                                <span
                                  class="mr-1"
                                  classList={{
                                    "text-rk-green": f.staged,
                                    "text-rk-red": f.unstaged && !f.staged,
                                    "text-rk-muted": f.untracked,
                                  }}
                                >
                                  {f.untracked ? "?" : f.staged ? "+" : "•"}
                                </span>
                                {f.path}
                              </button>
                            )}
                          </For>
                        </Show>
                      </div>
                      <div class="min-w-0 flex-1 overflow-auto p-2">
                        <Show
                          when={gitDiff()}
                          fallback={
                            <p class="text-xs text-rk-muted">
                              Select a file to view its diff.
                            </p>
                          }
                        >
                          <DiffView diff={gitDiff()} />
                        </Show>
                      </div>
                    </div>
                  </>
                );
              })()}
            </Show>
          </div>
        </Show>

        <Show when={store.activeTab() === "memory"}>
          <div class="absolute inset-0 overflow-y-auto p-3">
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
          </div>
        </Show>

        <Show when={store.activeTab() === "web"}>
          <div class="absolute inset-0 overflow-y-auto p-3">
            <p class="text-xs text-rk-muted">
              Web fetch/search results render here when the agent browses.
            </p>
          </div>
        </Show>
      </div>
    </div>
  );
}
