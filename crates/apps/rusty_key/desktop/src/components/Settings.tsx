import { createSignal, For, onMount, Show } from "solid-js";
import { ipc } from "../ipc";
import { asBoundaryError } from "../ipc";
import { store } from "../store";
import { applyTheme, loadTheme, THEMES } from "../theme";

/// Settings (PRD 08). API keys round-trip the OS keychain via `secrets_*` — they
/// never live in localStorage. MCP management, harness tuning, and themes deepen
/// in the settings pass; v1 covers config display, keychain keys, and MCP list.
export default function Settings(props: { onClose: () => void }) {
  const [provider, setProvider] = createSignal("openai");
  const [keyInput, setKeyInput] = createSignal("");
  const [keyStatus, setKeyStatus] = createSignal("");
  const [mcp, setMcp] = createSignal<{ name: string; tool_count: number }[]>([]);
  const [theme, setTheme] = createSignal(loadTheme());
  const [tuneStatus, setTuneStatus] = createSignal("");

  const TUNING: { key: string; label: string; placeholder: string }[] = [
    { key: "recall_k", label: "Recall K", placeholder: "e.g. 5" },
    { key: "idle_threshold", label: "Idle threshold", placeholder: "e.g. 8" },
    { key: "harness_level", label: "Harness level", placeholder: "H1 / H2 / H3" },
    { key: "compact_micro", label: "Compact micro", placeholder: "0.80" },
  ];

  const setTuning = async (key: string, value: string) => {
    if (!value.trim()) return;
    try {
      await ipc.configSet(key, value);
      setTuneStatus(`Recorded ${key} = ${value} (restart-only keys apply next launch).`);
    } catch (err) {
      setTuneStatus(`Error: ${asBoundaryError(err).message}`);
    }
  };

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

        <section class="mb-4 rounded border border-rk-border bg-rk-panel p-3">
          <h3 class="mb-2 text-sm font-semibold text-rk-gold">Harness Tuning</h3>
          <div class="grid grid-cols-1 gap-2 sm:grid-cols-2">
            <For each={TUNING}>
              {(t) => (
                <label class="flex items-center gap-2 text-xs">
                  <span class="w-28 text-rk-muted">{t.label}</span>
                  <input
                    class="flex-1 rounded border border-rk-border bg-rk-bg px-2 py-1 text-rk-text"
                    placeholder={t.placeholder}
                    onChange={(e) => setTuning(t.key, e.currentTarget.value)}
                  />
                </label>
              )}
            </For>
          </div>
          <Show when={tuneStatus()}>
            <p class="mt-2 text-xs text-rk-muted">{tuneStatus()}</p>
          </Show>
        </section>

        <section class="rounded border border-rk-border bg-rk-panel p-3">
          <h3 class="mb-2 text-sm font-semibold text-rk-gold">Appearance</h3>
          <label class="flex items-center gap-2 text-xs">
            <span class="w-28 text-rk-muted">Theme</span>
            <select
              class="rounded border border-rk-border bg-rk-bg px-2 py-1 text-rk-text"
              value={theme()}
              onChange={(e) => {
                setTheme(e.currentTarget.value);
                applyTheme(e.currentTarget.value);
              }}
            >
              <For each={THEMES}>{(t) => <option value={t.name}>{t.name}</option>}</For>
            </select>
          </label>
        </section>
      </div>
    </div>
  );
}
