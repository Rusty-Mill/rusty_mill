import { createEffect, onCleanup, onMount } from "solid-js";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";
import { store } from "../store";

/// xterm.js view of the `rk://bash_output` stream (PRD 08). On (re)mount it
/// replays the full chunk buffer, so switching away and back keeps history.
export default function TerminalView() {
  let el: HTMLDivElement | undefined;
  let term: Terminal | undefined;
  let fit: FitAddon | undefined;
  let written = 0;

  const safeFit = () => {
    try {
      fit?.fit();
    } catch (_) {
      /* container not laid out yet */
    }
  };

  const flush = () => {
    const chunks = store.terminalChunks;
    for (let i = written; i < chunks.length; i++) term?.write(chunks[i]);
    written = chunks.length;
  };

  onMount(() => {
    term = new Terminal({
      convertEol: true,
      fontSize: 12,
      fontFamily: 'ui-monospace, "JetBrains Mono", Menlo, monospace',
      theme: { background: "#0f1115", foreground: "#d7dce5", cursor: "#d4a017" },
    });
    fit = new FitAddon();
    term.loadAddon(fit);
    if (el) {
      term.open(el);
      safeFit();
    }
    flush();
    const onResize = () => safeFit();
    window.addEventListener("resize", onResize);
    onCleanup(() => {
      window.removeEventListener("resize", onResize);
      term?.dispose();
    });
  });

  createEffect(() => {
    void store.terminalChunks.length;
    flush();
    safeFit();
  });

  return <div ref={el} class="h-full w-full" />;
}
