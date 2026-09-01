import { respondApproval } from "../store";
import type { ApprovalRequest } from "../contract";

/// Replaces the composer when `rk://approval_request` fires. Block records a
/// `tool_block` intervention on the Rust side.
export default function ApprovalGate(props: { request: ApprovalRequest }) {
  const argText = () => {
    try {
      return JSON.stringify(props.request.args);
    } catch (_) {
      return String(props.request.args);
    }
  };
  return (
    <div class="border-t border-rk-yellow/50 bg-rk-yellow/10 px-4 py-3">
      <div class="mb-2 text-sm text-rk-yellow">⚠ Agent wants to run:</div>
      <div class="mb-3 mono text-sm text-rk-text">
        {props.request.tool}(<span class="text-rk-muted">{argText()}</span>)
      </div>
      <div class="flex gap-2">
        <button
          class="rounded bg-rk-green/20 px-3 py-1 text-sm text-rk-green hover:bg-rk-green/30"
          onClick={() => respondApproval(true, false)}
        >
          Allow
        </button>
        <button
          class="rounded bg-rk-green/10 px-3 py-1 text-sm text-rk-green hover:bg-rk-green/20"
          onClick={() => respondApproval(true, true)}
        >
          Allow Always
        </button>
        <button
          class="rounded bg-rk-red/20 px-3 py-1 text-sm text-rk-red hover:bg-rk-red/30"
          onClick={() => respondApproval(false, false)}
        >
          Block
        </button>
      </div>
    </div>
  );
}
