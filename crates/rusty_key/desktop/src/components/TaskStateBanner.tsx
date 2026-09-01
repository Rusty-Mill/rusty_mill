import { Show } from "solid-js";
import { store } from "../store";

/// Persistent strip above the conversation when a task is active. v1 derives the
/// goal from config when available; the full TaskState drawer (criteria checklist)
/// lands with a dedicated task command in a follow-on.
export default function TaskStateBanner() {
  const goal = () => {
    const cfg = store.config() as { task_goal?: string } | null;
    return cfg?.task_goal ?? "";
  };
  return (
    <Show when={goal()}>
      <div class="mb-2 flex items-center gap-2 rounded border border-rk-border bg-rk-panel-2 px-3 py-1.5 text-xs">
        <span class="rounded bg-rk-gold/20 px-1.5 py-0.5 text-rk-gold">Task</span>
        <span class="truncate text-rk-text">{goal()}</span>
      </div>
    </Show>
  );
}
