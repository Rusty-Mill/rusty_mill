import { createEffect, For, Show } from "solid-js";
import { store } from "../store";
import TurnCard from "./TurnCard";
import TaskStateBanner from "./TaskStateBanner";

/// The primary panel: the conversation as a list of TurnCards (newest at bottom),
/// with a live card while a turn is running.
export default function SessionPanel() {
  let scroller: HTMLDivElement | undefined;

  // Auto-scroll to the newest turn / streaming tokens.
  createEffect(() => {
    void store.turns.length;
    void store.pendingTokens();
    void store.pendingTools.length;
    queueMicrotask(() => {
      if (scroller) scroller.scrollTop = scroller.scrollHeight;
    });
  });

  return (
    <div class="flex h-full flex-col bg-rk-bg">
      <div class="shrink-0 border-b border-rk-border px-4 pt-3">
        <TaskStateBanner />
      </div>
      <div ref={scroller} class="min-h-0 flex-1 overflow-y-auto px-4 py-3">
        <Show
          when={store.turns.length > 0 || store.locked()}
          fallback={
            <div class="mt-12 text-center text-rk-muted">
              <p class="text-lg">Start a turn</p>
              <p class="mt-1 text-xs">
                Type a message below. Tool calls, verification, and entropy appear here.
              </p>
            </div>
          }
        >
          <For each={store.turns}>{(turn) => <TurnCard turn={turn} />}</For>
          <Show when={store.locked()}>
            <TurnCard
              pending
              pendingTokens={store.pendingTokens()}
              pendingTools={[...store.pendingTools]}
            />
          </Show>
        </Show>
      </div>
    </div>
  );
}
