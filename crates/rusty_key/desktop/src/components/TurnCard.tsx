import { createSignal, For, Show } from "solid-js";
import type { Turn } from "../store";
import type { ToolEvent } from "../contract";
import ToolEventRow from "./ToolEventRow";
import VerificationBadge from "./VerificationBadge";

/// A single turn: user message, collapsible tool trace, reply, verdict, limits.
/// When `pending`, it renders the live streaming tokens/tools instead of a verdict.
export default function TurnCard(props: {
  turn?: Turn;
  pending?: boolean;
  pendingTokens?: string;
  pendingTools?: ToolEvent[];
  userMessage?: string;
}) {
  const [expanded, setExpanded] = createSignal(true);
  const tools = (): ToolEvent[] => props.turn?.tool_events ?? props.pendingTools ?? [];
  const userMsg = () => props.turn?.user_message ?? props.userMessage ?? "";
  const reply = () => props.turn?.reply ?? props.pendingTokens ?? "";

  return (
    <div class="mb-3 rounded-md border border-rk-border bg-rk-panel">
      <div class="flex items-center justify-between border-b border-rk-border px-3 py-1.5">
        <button
          class="mono text-xs text-rk-muted hover:text-rk-text"
          onClick={() => setExpanded((v) => !v)}
        >
          {expanded() ? "▾" : "▸"} {props.turn ? `Turn ${props.turn.id}` : "Turn (running)"}
        </button>
        <VerificationBadge verified={props.turn?.verified} pending={props.pending} />
      </div>

      <div class="px-3 py-2">
        <Show when={userMsg()}>
          <div class="mb-2 text-rk-text">
            <span class="text-rk-muted">User:</span> {userMsg()}
          </div>
        </Show>

        <Show when={expanded() && tools().length > 0}>
          <div class="mb-2 rounded bg-rk-bg/50 px-2 py-1">
            <For each={tools()}>{(ev) => <ToolEventRow event={ev} />}</For>
          </div>
        </Show>

        <Show when={reply()}>
          <div class="whitespace-pre-wrap text-rk-text">{reply()}</div>
        </Show>

        <Show when={props.turn}>
          {(t) => (
            <div class="mt-2 border-t border-rk-border pt-1.5 mono text-xs text-rk-muted">
              Limits: {t().limits}
            </div>
          )}
        </Show>
      </div>
    </div>
  );
}
