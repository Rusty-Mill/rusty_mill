// Code view: a line-number gutter and Markdown/frontmatter colouring for the editor, built with
// DOM nodes and classes only (no inline styles, no HTML strings), so the strict CSP stays as is.
// `tokenize` is pure and also loads under Node for tests.
(function () {
  const MAX_LINES = 4000; // past this a file is shown plain: colouring must never slow typing

  const MD = /\.(md|markdown)$/i;
  const KEY = /^([A-Za-z0-9_-]+)(:)(.*)$/;
  const INLINE = /(`[^`\n]+`)|(\*\*[^*\n]+\*\*)/g;

  // Splits plain text into [class, text] parts: code spans and bold.
  function inline(s) {
    const out = [];
    let at = 0;
    for (const m of s.matchAll(INLINE)) {
      if (m.index > at) out.push(["", s.slice(at, m.index)]);
      out.push([m[1] ? "tok-code" : "tok-bold", m[0]]);
      at = m.index + m[0].length;
    }
    if (at < s.length) out.push(["", s.slice(at)]);
    return out;
  }

  // One array of [class, text] parts per line; joining the texts gives the input back.
  function tokenize(text, path) {
    const lines = text.split("\n");
    if (!MD.test(path ?? "") || lines.length > MAX_LINES) return lines.map((l) => (l ? [["", l]] : []));
    let front = lines[0] === "---" ? "open" : "none"; // frontmatter: open, then done
    let fence = false;
    return lines.map((l, i) => {
      if (front === "open") {
        if (i > 0 && l === "---") { front = "done"; return [["tok-meta", l]]; }
        if (i === 0) return [["tok-meta", l]];
        const m = KEY.exec(l);
        return m ? [["tok-key", m[1]], ["tok-meta", m[2]], ["", m[3]]] : l ? [["", l]] : [];
      }
      if (/^(```|~~~)/.test(l)) { fence = !fence; return [["tok-fence", l]]; }
      if (fence) return l ? [["tok-code", l]] : [];
      if (/^#{1,6}\s/.test(l)) return [["tok-head", l]];
      if (/^>/.test(l)) return [["tok-quote", l]];
      if (/^(-{3,}|\*{3,}|_{3,})\s*$/.test(l)) return [["tok-meta", l]];
      const list = /^(\s*(?:[-*+]|\d+[.)])\s)(.*)$/.exec(l);
      if (list) return [["tok-mark", list[1]], ...inline(list[2])];
      return inline(l);
    });
  }

  // Fills `layer` with coloured spans, one text node per line break, so its textContent is `text`.
  function paint(layer, text, path) {
    const frag = document.createDocumentFragment();
    const lines = tokenize(text, path);
    lines.forEach((parts, i) => {
      for (const [cls, t] of parts) {
        if (!cls) frag.appendChild(document.createTextNode(t));
        else {
          const s = document.createElement("span");
          s.className = cls;
          s.textContent = t;
          frag.appendChild(s);
        }
      }
      if (i < lines.length - 1) frag.appendChild(document.createTextNode("\n"));
    });
    layer.replaceChildren(frag);
  }

  // Line numbers for `text`; the class sets the gutter width for the digit count.
  function number(gutter, text) {
    const n = text.split("\n").length;
    gutter.textContent = Array.from({ length: n }, (_, i) => i + 1).join("\n");
    gutter.className = `ed-gutter d${Math.min(String(n).length, 6)}`;
  }

  const api = { tokenize, paint, number, MAX_LINES };
  if (typeof module !== "undefined") module.exports = api;
  else window.Code = api;
})();
