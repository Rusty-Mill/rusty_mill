import { createSignal, For, Show, onMount } from "solid-js";
import { sendTurn, store } from "../store";
import { ipc } from "../ipc";
import ApprovalGate from "./ApprovalGate";
import PlanConfirm from "./PlanConfirm";

type PickerKind = "file" | "memory" | "command";

const HISTORY_KEY = "rk.composer.history";

export default function Composer() {
  const [message, setMessage] = createSignal("");
  const [attachments, setAttachments] = createSignal<string[]>([]);
  const [picker, setPicker] = createSignal<PickerKind | null>(null);
  const [options, setOptions] = createSignal<string[]>([]);
  const [history, setHistory] = createSignal<string[]>([]);
  const [histIdx, setHistIdx] = createSignal(-1);
  let textarea: HTMLTextAreaElement | undefined;

  onMount(() => {
    try {
      const raw = localStorage.getItem(HISTORY_KEY);
      if (raw) setHistory(JSON.parse(raw));
    } catch (_) {
      /* ignore corrupt history */
    }
  });

  const pushHistory = (msg: string) => {
    const next = [...history().filter((h) => h !== msg), msg].slice(-100);
    setHistory(next);
    setHistIdx(-1);
    try {
      localStorage.setItem(HISTORY_KEY, JSON.stringify(next));
    } catch (_) {
      /* storage may be unavailable */
    }
  };

  const closePicker = () => {
    setPicker(null);
    setOptions([]);
  };

  const openPicker = async (kind: PickerKind, tok: string) => {
    setPicker(kind);
    const low = tok.toLowerCase();
    try {
      if (kind === "file") {
        const files = await ipc.fsListWorkspace();
        setOptions(files.filter((f) => f.toLowerCase().includes(low)).slice(0, 30));
      } else if (kind === "memory") {
        const mems = await ipc.memorySearch(tok);
        setOptions(mems.map((m) => m.title).slice(0, 30));
      } else {
        const cmds = await ipc.commandsList();
        setOptions(cmds.filter((c) => c.toLowerCase().includes(low)).slice(0, 30));
      }
    } catch (_) {
      setOptions([]);
    }
  };

  // Detect a trigger token at the caret and (re)open the matching picker.
  const onInput = (value: string) => {
    setMessage(value);
    const m = /(^|\s)([@#/])([^\s]*)$/.exec(value);
    if (!m) {
      if (picker()) closePicker();
      return;
    }
    const kind: PickerKind =
      m[2] === "@" ? "file" : m[2] === "#" ? "memory" : "command";
    void openPicker(kind, m[3]);
  };

  const applyOption = (opt: string) => {
    const kind = picker();
    const value = message();
    const replaced = value.replace(/([@#/])([^\s]*)$/, (_full, sigil) => {
      if (kind === "file") {
        setAttachments((a) => (a.includes(opt) ? a : [...a, opt]));
        return `${sigil}${opt} `;
      }
      return `${sigil}${opt} `;
    });
    setMessage(replaced);
    closePicker();
    textarea?.focus();
  };

  const submit = () => {
    const msg = message().trim();
    if (!msg || store.locked()) return;
    pushHistory(msg);
    void sendTurn(msg, attachments());
    setMessage("");
    setAttachments([]);
    closePicker();
  };

  const onKeyDown = (e: KeyboardEvent) => {
    if (picker() && (e.key === "Escape")) {
      e.preventDefault();
      closePicker();
      return;
    }
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      submit();
      return;
    }
    if (!picker() && e.key === "ArrowUp" && message() === "") {
      e.preventDefault();
      const h = history();
      if (!h.length) return;
      const idx = histIdx() < 0 ? h.length - 1 : Math.max(0, histIdx() - 1);
      setHistIdx(idx);
      setMessage(h[idx]);
    }
    if (!picker() && e.key === "ArrowDown" && histIdx() >= 0) {
      e.preventDefault();
      const h = history();
      const idx = histIdx() + 1;
      if (idx >= h.length) {
        setHistIdx(-1);
        setMessage("");
      } else {
        setHistIdx(idx);
        setMessage(h[idx]);
      }
    }
  };

  return (
    <div class="shrink-0">
      <Show when={store.approval()}>
        {(req) => <ApprovalGate request={req()} />}
      </Show>
      <Show when={store.planExit()}>{(plan) => <PlanConfirm plan={plan()} />}</Show>

      <Show when={!store.approval() && !store.planExit()}>
        <div class="relative border-t border-rk-border bg-rk-panel px-4 py-2">
          <Show when={picker() && options().length > 0}>
            <div class="absolute bottom-full left-4 right-4 mb-1 max-h-60 overflow-y-auto rounded border border-rk-border bg-rk-panel-2 shadow-lg">
              <For each={options()}>
                {(opt) => (
                  <button
                    class="block w-full truncate px-3 py-1 text-left mono text-xs text-rk-text hover:bg-rk-gold/20"
                    onClick={() => applyOption(opt)}
                  >
                    {opt}
                  </button>
                )}
              </For>
            </div>
          </Show>

          <Show when={attachments().length > 0}>
            <div class="mb-1 flex flex-wrap gap-1">
              <For each={attachments()}>
                {(a) => (
                  <span class="rounded bg-rk-panel-2 px-1.5 py-0.5 mono text-xs text-rk-blue">
                    @{a}
                    <button
                      class="ml-1 text-rk-muted hover:text-rk-red"
                      onClick={() => setAttachments((list) => list.filter((x) => x !== a))}
                    >
                      ×
                    </button>
                  </span>
                )}
              </For>
            </div>
          </Show>

          <textarea
            ref={textarea}
            class="h-16 w-full resize-none rounded border border-rk-border bg-rk-bg px-3 py-2 text-rk-text placeholder:text-rk-muted focus:border-rk-gold focus:outline-none disabled:opacity-50"
            classList={{ "opacity-50": store.locked() }}
            placeholder={
              store.locked()
                ? "Turn running…"
                : "Message the agent — @file, #memory, /command. Enter to send."
            }
            value={message()}
            disabled={store.locked()}
            onInput={(e) => onInput(e.currentTarget.value)}
            onKeyDown={onKeyDown}
          />
        </div>
      </Show>
    </div>
  );
}
