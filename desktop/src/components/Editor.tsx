import { createEffect, onCleanup } from "solid-js";
import { EditorState, type Extension } from "@codemirror/state";
import { EditorView, lineNumbers } from "@codemirror/view";
import { MergeView } from "@codemirror/merge";
import { oneDark } from "@codemirror/theme-one-dark";
import { langForPath } from "../lang";

/// CodeMirror 6 view of the latest edit (PRD 08, diff-first). An `edit_file`
/// (old_string → new_string) renders as a side-by-side MergeView; a `write_file`
/// renders its content read-only with syntax highlighting.
export default function Editor(props: {
  path?: string;
  content?: string;
  oldString?: string;
  newString?: string;
}) {
  let host: HTMLDivElement | undefined;
  let view: EditorView | MergeView | undefined;

  const base = (path?: string): Extension[] => [
    lineNumbers(),
    oneDark,
    EditorView.editable.of(false),
    EditorState.readOnly.of(true),
    EditorView.lineWrapping,
    ...langForPath(path),
  ];

  createEffect(() => {
    const { path, content, oldString, newString } = props;
    if (!host) return;
    view?.destroy();
    host.innerHTML = "";
    if (oldString !== undefined && newString !== undefined) {
      view = new MergeView({
        a: { doc: oldString, extensions: base(path) },
        b: { doc: newString, extensions: base(path) },
        parent: host,
        gutter: true,
      });
    } else {
      view = new EditorView({
        state: EditorState.create({ doc: content ?? "", extensions: base(path) }),
        parent: host,
      });
    }
  });

  onCleanup(() => view?.destroy());

  return <div ref={host} class="h-full w-full overflow-auto text-xs" />;
}
