import { Show } from "solid-js";

/// ✓ green / ✗ red verdict, or a spinner while a turn is pending.
export default function VerificationBadge(props: {
  verified?: boolean;
  pending?: boolean;
}) {
  return (
    <Show
      when={!props.pending}
      fallback={
        <span class="mono text-xs text-rk-muted">
          <span class="inline-block animate-spin">◌</span> running
        </span>
      }
    >
      <Show
        when={props.verified}
        fallback={<span class="mono text-xs font-semibold text-rk-red">✗ UNVERIFIED</span>}
      >
        <span class="mono text-xs font-semibold text-rk-green">✓ VERIFIED</span>
      </Show>
    </Show>
  );
}
