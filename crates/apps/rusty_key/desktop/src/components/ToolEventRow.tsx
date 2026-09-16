import { Show } from "solid-js";
import type { ToolEvent } from "../contract";

function statusGlyph(status: string): { glyph: string; color: string } {
  switch (status) {
    case "ok":
      return { glyph: "✓", color: "text-rk-green" };
    case "blocked":
      return { glyph: "⛔", color: "text-rk-yellow" };
    case "timeout":
      return { glyph: "⏱", color: "text-rk-yellow" };
    case "truncated":
      return { glyph: "✂", color: "text-rk-yellow" };
    default:
      return { glyph: "✗", color: "text-rk-red" };
  }
}

function argSummary(args: unknown): string {
  if (args && typeof args === "object") {
    const obj = args as Record<string, unknown>;
    const primary = obj.path ?? obj.command ?? obj.query ?? obj.pattern;
    if (primary !== undefined) return String(primary);
    const keys = Object.keys(obj);
    return keys.length ? `${keys[0]}=${String(obj[keys[0]])}` : "";
  }
  return args === undefined ? "" : String(args);
}

/// One `▸ tool("arg")  ✓` line within a TurnCard's trace.
export default function ToolEventRow(props: { event: ToolEvent }) {
  const status = () => statusGlyph(props.event.outcome?.status ?? "error");
  return (
    <div class="flex items-center gap-2 py-0.5 mono text-xs">
      <span class="text-rk-muted">▸</span>
      <span class="text-rk-blue">{props.event.name}</span>
      <span class="truncate text-rk-muted">({argSummary(props.event.args)})</span>
      <span class={status().color}>{status().glyph}</span>
      <Show when={props.event.outcome?.status && props.event.outcome.status !== "ok"}>
        <span class="text-rk-muted">{props.event.outcome.status}</span>
      </Show>
    </div>
  );
}
