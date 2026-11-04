---
name: rusty-mux
description: Drive the rusty_mux terminal multiplexer with rusty-mux-ctl. Use when RUSTY_MUX_BLOCK is set, or when asked to run, watch or read programs in rusty_mux sessions, windows or blocks.
---

<!-- Installed by `rusty-mux-ctl agent setup`; `rusty-mux-ctl agent remove` deletes it. -->

# rusty_mux

The guide to rusty_mux comes from the installed `rusty-mux-ctl`, so it always
matches the version you drive. Before using rusty_mux, run

```sh
rusty-mux-ctl agent skill
```

and follow what it prints. If the session-start message names where
rusty-mux-ctl is, run that path instead.

If the shell cannot find `rusty-mux-ctl`, tell the user that rusty_mux's ctl
is not on PATH. Do not guess its commands from memory or from another copy of
this guide.
