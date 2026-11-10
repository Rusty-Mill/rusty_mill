---
category: Added
changelog: rusty_term and rusty_mux: `grapheme_clustering` config key starts new panes with DEC mode 2027 set; off by default.
---
## 2026-11-10 - Config key for mode 2027 grapheme clustering

- **Added:** `grapheme_clustering = true` in rusty_term's config starts the TUI's pane with DEC mode 2027 (grapheme clustering) set. The mux config's `server` section takes `"grapheme_clustering": true` for new blocks, read at start and on `reload-config`; this is the key GUI users set, since `rusty_term --gui` runs `rusty-mux-gui`, which warns when rusty_term's key is set. Off by default. The mode is set through the pane's parser, so DECRQM reports it, a program can reset it, and the attach checkpoint already carries it to every client: no wire change.
- **Known limitations:** RIS (`reset`) turns clustering off rather than back to the configured default. A reload changes new blocks only. An older server or ctl refuses a mux config that has the new key.
