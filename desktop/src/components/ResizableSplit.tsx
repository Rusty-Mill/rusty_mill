import { createSignal, type JSX } from "solid-js";

/// A horizontal two-pane split with a draggable divider. `initial`/`min`/`max`
/// are fractions of the total width assigned to the left pane.
export function ResizableSplit(props: {
  left: JSX.Element;
  right: JSX.Element;
  initial?: number;
  min?: number;
  max?: number;
}) {
  const [frac, setFrac] = createSignal(props.initial ?? 0.5);
  let container: HTMLDivElement | undefined;

  const onDown = (e: MouseEvent) => {
    e.preventDefault();
    const move = (ev: MouseEvent) => {
      if (!container) return;
      const rect = container.getBoundingClientRect();
      const f = (ev.clientX - rect.left) / rect.width;
      const min = props.min ?? 0.2;
      const max = props.max ?? 0.8;
      setFrac(Math.min(max, Math.max(min, f)));
    };
    const up = () => {
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
      document.body.style.cursor = "";
    };
    document.body.style.cursor = "col-resize";
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
  };

  return (
    <div ref={container} class="flex h-full w-full">
      <div class="min-w-0 overflow-hidden" style={{ width: `${frac() * 100}%` }}>
        {props.left}
      </div>
      <div
        class="w-1 shrink-0 cursor-col-resize bg-rk-border transition-colors hover:bg-rk-gold"
        onMouseDown={onDown}
        role="separator"
        aria-orientation="vertical"
      />
      <div class="min-w-0 flex-1 overflow-hidden">{props.right}</div>
    </div>
  );
}
