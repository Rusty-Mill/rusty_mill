---
category: Changed
changelog: rusty_mux: the takeover screen test reports both clients' screens and the old client's raw output when it fails (test only).
---
## 2026-11-05 - Takeover screen test says what it saw

- **Changed:** `a_reconnect_keeps_the_client_its_screen_and_its_lease` keeps its assertion. On failure it now prints the checkpoint's screen, the old client's screen and its raw output. The test failed on Windows in #1532's queue run with no other output, and this tells a checkpoint that lost the text from a terminal that no longer showed it.
- **Known limitations:** diagnostics only; no behaviour change.
