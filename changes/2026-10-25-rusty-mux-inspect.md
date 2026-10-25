---
category: Added
changelog: rusty_mux list and capture requests, to read sessions, windows, blocks and a block's screen without attaching.
---
## 2026-10-25 - rusty_mux list and capture

- **Added:** two read-only requests, negotiated with the new feature bit `Features::INSPECT` (bit 6). Both are in `schema::CATALOG`, so `rusty-mux-ctl` gains `list` and `capture` without new client code.
  - `List` (type 0x0e) is answered with one `Entry` (0x8a) per session, window and block, each parent before its children, then `Done`. A block entry carries its window, its size, the lease holder, and the title its program set.
  - `Capture` (type 0x0d) is answered with `Text` (0x89): the last `history` scrollback lines, then the screen, one line per row with trailing blanks trimmed. A reply to either over 4 MiB encoded is refused whole with `Failed`, before any of it is sent, so a reply alone can't exceed a client's output budget.
  - The server refuses both, and `attach`, from a client that did not negotiate the feature they need.
- **Added:** `schema::Method::needs`, the features a method needs; `Returns::Text` and `Returns::Entries`; `Ty::Lines` (a count that may be 0). ctl offers every method's features in Hello, and refuses a chain the server can't run before sending any of it. In ctl's text output, names and titles are quoted and escaped, so one entity is one line and control characters never reach the terminal.
- **Added:** `rusty_term_core::Grid::line_text`, the text of a line by its position counting scrollback and the screen, available without the `gui` feature.
- **Known limitations:**
  - `capture` gives plain text, without colors or attributes.
  - On the alternate screen it shows that screen; the scrollback is the primary screen's.
