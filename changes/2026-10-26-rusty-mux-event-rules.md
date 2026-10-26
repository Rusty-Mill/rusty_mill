---
category: Added
changelog: rusty_mux event rules: the config's `on` rules run an action or request when an event happens, with the event's fields such as `${block}` or `${pwd}`; `rusty-mux-ctl watch` runs them, and the server now announces working-directory and title changes.
---
## 2026-10-26 - rusty_mux event rules (C19)

- **Added:** `on` rules in the rusty_mux config, the declarative form of Rex's `rex.on` hooks. A rule is `{"event", "action", "args"?}`: an event kind as `rusty-mux-ctl events` prints it, and an action or catalog request whose values may hold the event's fields, such as `${block}`, `${session}`, `${cols}`, `${pwd}` or `${title}`. `config check` reports an unknown event, a field the event lacks, and anything a binding would get wrong; a value with a field is checked once the event fills it. `config list` prints the rules too.
- **Added:** `rusty-mux-ctl watch [--after EPOCH:SEQ]` follows events and runs every matching rule, in order, each on a connection of its own as `rusty-mux-ctl` would run it. It prints the start cursor, each matched event and what its rules print. A rule that fails is reported on stderr and the watch goes on. For `pwd-changed` and `title-changed` it reads the new directory or title from the block first.
- **Added:** `Event::PwdChanged` (kind 14) and `Event::TitleChanged` (kind 15). The server emits them when a block's program reports a new working directory with OSC 7 or sets a new title. They reach only clients that negotiate `Features::TERMINAL_EVENTS` (bit 9), live and on replay. `rusty-mux-ctl events` asks for them and prints `pwd-changed` and `title-changed`.
- **Known limitations:** `watch` reads the config once when it starts, so restart it after an edit. A rule that causes its own event, such as one creating a block on `block-created`, runs again each time. No bell or program-exit events yet, and no timers like `rex.after`. A directory found only through `/proc`, not OSC 7, raises no event.
