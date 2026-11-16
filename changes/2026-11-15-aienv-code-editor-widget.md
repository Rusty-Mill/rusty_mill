---
category: Added
changelog: The Ai Env Manager editor shows line numbers and colours Markdown and frontmatter, with no new dependency.
---
## 2026-11-15 - Code view in the Ai Env Manager editor

- **Added:** the GUI skill-file editor has a line-number gutter and colours frontmatter keys, headings, fenced and inline code, lists, quotes and bold in Markdown files (`ui/code.js`). The editor is still a plain `<textarea>`, laid over a coloured copy of its text. No npm or crate dependency, and the CSP is unchanged (no inline styles, no workers); text is set with `textContent` only (D43).
- **Known limitations:** only Markdown is coloured, and files over 4000 lines are shown plain; lines do not wrap; no search, undo history beyond the textarea's own, or bracket matching. Checked with a standalone Playwright script (`gui/tests/ui/code-widget.cjs`) that CI does not run; no real Tauri window run yet.
