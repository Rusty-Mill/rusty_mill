# rusty_term GUI design

Design for the GUI that rusty_term and rusty_mux share: one window for sessions, windows and
split blocks, including blocks driven by agents. Frames, not code. Implementing them in
`crates/libs/ui/rusty_term` is a later handoff.

Start with [SPEC.md](SPEC.md), then [DECISIONS.md](DECISIONS.md) (D1 to D30) and
[QA.md](QA.md). Open questions are in [OPEN_QUESTIONS.md](OPEN_QUESTIONS.md); Q3 (does Take over
confirm first) is waiting for the owner.

## Where the design lives

- **Paper file "rusty_term GUI"** (<https://app.paper.design/file/01M6H59EEHRED6B08WXEEX8WZ5>):
  F1 to F8 as built frames, in one row. F8 holds the tokens and components. The partial pages
  there (States and variants, Light theme, Alt layouts, Settings) stopped at Paper's weekly MCP
  limit; the HTML pages below finish them.
- **`assets/html/`**: standalone pages that cover the rest. They use the same tokens, and the
  light theme is the same markup with the tokens swapped (D24, D25).

| Page | Contents |
|---|---|
| `states.html` | control, input, run, header, tab and banner states; reduced motion; windows 2 and 3; split and close flows; empty window; four error states |
| `light.html` | light tokens and components; light F1, F2, F3a, F3b, F4, F5a, F5b, F6, F7 |
| `alt-layouts.html` | sidebar tree (open and collapsed), bottom dock (with switcher), comparison |
| `lel.html` (D37 to D41) | Let Em Loose tab: L1 full-app view dark and light, L2 inbox, L3 states |
| `settings.html` | settings overlay for each section, plus every control in its states |

Open `assets/html/index.html`. The pages were checked structurally (every tag closes), but no
browser was available when they were written, so they have not been looked at rendered. Treat
visual details as unconfirmed.

## Rebuild the HTML

```
cd docs/projects/term/gui-design/assets/html
python build_gui.py
```

Fonts are not committed. Fetch them once, next to the pages, or the pages fall back to system
fonts:

```
mkdir -p fonts && cd fonts
curl -LO https://cdn.jsdelivr.net/npm/@fontsource-variable/instrument-sans/files/instrument-sans-latin-wght-normal.woff2
curl -LO https://cdn.jsdelivr.net/npm/@fontsource-variable/jetbrains-mono/files/jetbrains-mono-latin-wght-normal.woff2
```

Both families are SIL Open Font License.

## Other files

- `assets/terminal-mux.svg` and `.png` are the v2 concept. `build_mux.py` holds its exact values.
  `terminal-modern.svg` is v1 and `terminal.svg` is v0 (old palette, rejected by D5 and D7).
- `CLAUDE.snippet.md` is the rule to merge into `CLAUDE.md` when work on this starts: questions
  go to the `rusty-term-design-expert` subagent first. The agent is
  `.claude/agents/rusty-term-design-expert.md`.
