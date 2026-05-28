import { createSignal, Show } from "solid-js";
import { resolvePlan } from "../store";

/// Renders when `rk://plan_exit` fires: confirm, reject, or annotate the plan.
/// Annotation is sent as a follow-up turn.
export default function PlanConfirm(props: { plan: string }) {
  const [annotating, setAnnotating] = createSignal(false);
  const [note, setNote] = createSignal(props.plan);

  return (
    <div class="border-t border-rk-blue/50 bg-rk-blue/10 px-4 py-3">
      <div class="mb-2 text-sm text-rk-blue">Agent has proposed a plan. Proceed?</div>
      <Show
        when={!annotating()}
        fallback={
          <div>
            <textarea
              class="mb-2 h-32 w-full resize-none rounded border border-rk-border bg-rk-bg px-2 py-1 mono text-xs text-rk-text"
              value={note()}
              onInput={(e) => setNote(e.currentTarget.value)}
            />
            <div class="flex gap-2">
              <button
                class="rounded bg-rk-blue/20 px-3 py-1 text-sm text-rk-blue hover:bg-rk-blue/30"
                onClick={() => resolvePlan("reject", note())}
              >
                Send annotation
              </button>
              <button
                class="rounded bg-rk-panel-2 px-3 py-1 text-sm text-rk-muted hover:text-rk-text"
                onClick={() => setAnnotating(false)}
              >
                Cancel
              </button>
            </div>
          </div>
        }
      >
        <pre class="mb-3 max-h-40 overflow-y-auto whitespace-pre-wrap rounded bg-rk-bg px-2 py-1 mono text-xs text-rk-text">
          {props.plan}
        </pre>
        <div class="flex gap-2">
          <button
            class="rounded bg-rk-green/20 px-3 py-1 text-sm text-rk-green hover:bg-rk-green/30"
            onClick={() => resolvePlan("proceed")}
          >
            Proceed
          </button>
          <button
            class="rounded bg-rk-red/20 px-3 py-1 text-sm text-rk-red hover:bg-rk-red/30"
            onClick={() => resolvePlan("reject")}
          >
            Reject
          </button>
          <button
            class="rounded bg-rk-panel-2 px-3 py-1 text-sm text-rk-muted hover:text-rk-text"
            onClick={() => setAnnotating(true)}
          >
            Annotate…
          </button>
        </div>
      </Show>
    </div>
  );
}
