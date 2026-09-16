// Map a file path to a CodeMirror 6 language extension for syntax highlighting.

import { javascript } from "@codemirror/lang-javascript";
import { json } from "@codemirror/lang-json";
import { python } from "@codemirror/lang-python";
import { rust } from "@codemirror/lang-rust";
import type { Extension } from "@codemirror/state";

export function langForPath(path?: string): Extension[] {
  const ext = (path ?? "").split(".").pop()?.toLowerCase() ?? "";
  switch (ext) {
    case "rs":
      return [rust()];
    case "ts":
    case "tsx":
    case "js":
    case "jsx":
    case "mjs":
      return [javascript({ typescript: ext.startsWith("ts"), jsx: ext.endsWith("x") })];
    case "py":
      return [python()];
    case "json":
      return [json()];
    default:
      return [];
  }
}
