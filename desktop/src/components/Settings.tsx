import { createSignal, For, onMount, Show } from "solid-js";
import { ipc } from "../ipc";
import { asBoundaryError } from "../ipc";
import { store } from "../store";

/// Settings (PRD 08). API keys round-trip the OS keychain via `secrets_*` — they
/// never live in localStorage. MCP management, harness tuning, and themes deepen
/// in the settings pass; v1 covers config display, keychain keys, and MCP list.
export default function Settings(props: { onClose: () => void }) {
  const [provider, setProvider] = createSignal("openai");
  const [keyInput, setKeyInput] = createSignal("");
  const [keyStatus, setKeyStatus] = createSignal("");
  const [mcp, setMcp] = createSignal<{ name: string; tool_count: number }[]>([]);

  onMount(async () => {
    try {
      setMcp(await ipc.mcpServersList());
    } catch (_) {
      /* */
    }
  });

  const saveKey = async () => {
    try {
      await ipc.secretsSet(provider(), keyInput());
      setKeyInput("");
      setKeyStatus(`Saved key for ${provider()} to the OS keychain.`);
    } catch (err) {
      setKeyStatus(`Error: ${asBoundaryError(err).message}`);
    }
  };

  const deleteKey = async () => {
    try {
      await ipc.secretsDelete(provider());
      setKeyStatus(`Deleted key for ${provider()}.`);
    } catch (err) {
      setKeyStatus(`Error: ${asBoundaryError(err).message}`);
    }
  };

  return (
    <div class="absolute inset-0 z-20 overflow-y-auto bg-rk-bg/95 p-6">
      <div class="mx-auto max-w-3xl">
        <div class="mb-4 flex items-center justify-between">
          <h2 class="text-lg font-semibold">Settings</h2>
          <button class="rounded px-3 py-1 text-sm text-rk-muted hover:bg-rk-panel-2" onClick={props.onClose}>
            Close (Esc)
          </button>
        </div>

        <section class="mb-4 rounded border border-rk-border bg-rk-panel p-3">
          <h3 class="mb-2 text-sm font-semibold text-rk-gold">Session</h3>
          <Show when={store.config()} fallback={<p class="text-xs text-rk-muted">(loading)</p>}>
            {(cfg) => (
              <div class="mono text-xs text-rk-text">
                <div>permission mode: {cfg().permission_mode}</div>
                <div>isolation: {cfg().isolation}</div>
                <div>explore: {cfg().explore_enabled ? "on" : "off"}</div>
              </div>
            )}
          </Show>
        </section>

        <section class="mb-4 rounded border border-rk-border bg-rk-panel p-3">
          <h3 class="mb-2 text-sm font-semibold text-rk-gold">API Keys (OS keychain)</h3>
          <div class="flex flex-wrap items-center gap-2">
            <input
              class="rounded border border-rk-border bg-rk-bg px-2 py-1 text-xs text-rk-text"
              value={provider()}
              onInput={(e) => setProvider(e.currentTarget.value)}
              placeholder="provider"
            />
            <input
              class="flex-1 rounded border border-rk-border bg-rk-bg px-2 py-1 text-xs text-rk-text"
              type="password"
              value={keyInput()}
              onInput={(e) => setKeyInput(e.currentTarget.value)}
              placeholder="API key (stored in keychain, never localStorage)"
            />
            <button
              class="rounded bg-rk-green/20 px-3 py-1 text-xs text-rk-green hover:bg-rk-green/30"
              onClick={saveKey}
            >
              Save
            </button>
            <button
              class="rounded bg-rk-red/20 px-3 py-1 text-xs text-rk-red hover:bg-rk-red/30"
              onClick={deleteKey}
            >
              Delete
            </button>
          </div>
          <Show when={keyStatus()}>
            <p class="mt-2 text-xs text-rk-muted">{keyStatus()}</p>
          </Show>
        </section>

        <section class="mb-4 rounded border border-rk-border bg-rk-panel p-3">
          <h3 class="mb-2 text-sm font-semibold text-rk-gold">MCP Servers</h3>
          <Show
            when={mcp().length}
            fallback={<p class="text-xs text-rk-muted">(no servers connected)</p>}
          >
            <For each={mcp()}>
              {(s) => (
                <div class="mono text-xs text-rk-text">
                  {s.name}: {s.tool_count} tools
                </div>
              )}
            </For>
          </Show>
          <p class="mt-2 text-xs text-rk-muted">
            Add/remove/test is restart-only in v1 (configure via RUSTYKEYS env + relaunch).
          </p>
        </section>

        <section class="rounded border border-rk-border bg-rk-panel p-3">
          <h3 class="mb-2 text-sm font-semibold text-rk-gold">Harness Tuning &amp; Appearance</h3>
          <p class="text-xs text-rk-muted">
            Model/provider picker, permissions, harness thresholds, and bundled themes land in the
            settings pass.
          </p>
        </section>
      </div>
    </div>
  );
}
