import { For } from "solid-js";

/// Render a unified `git diff` with +/- line coloring (read-only).
export default function DiffView(props: { diff: string }) {
  const lines = () => (props.diff ? props.diff.split("\n") : []);
  const cls = (l: string): string => {
    if (l.startsWith("+") && !l.startsWith("+++")) return "text-rk-green";
    if (l.startsWith("-") && !l.startsWith("---")) return "text-rk-red";
    if (l.startsWith("@@")) return "text-rk-blue";
    if (
      l.startsWith("diff ") ||
      l.startsWith("index ") ||
      l.startsWith("+++") ||
      l.startsWith("---")
    )
      return "text-rk-muted";
    return "text-rk-text";
  };
  return (
    <pre class="mono text-xs leading-tight">
      <For each={lines()}>{(l) => <div class={cls(l)}>{l || " "}</div>}</For>
    </pre>
  );
}
