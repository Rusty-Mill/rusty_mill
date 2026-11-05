# Arc 2: the GUI as a mux client (plan)

Roadmap arc 2, "One GUI for terminal and mux" ([ROADMAP.md](ROADMAP.md)). Owner GO: Nano,
2026-11-05 20:55Z, project thread message `cmsg_014f6o52E2iAAAgMm9v8hh1w3582EEHuK6PNVK64YWHa5q`
("1 go"). Nano restates it on this PR (D37).

Bare D-numbers are rows in [the handoff pack](../rusty_term/handoff/DECISIONS.md); "gui-design Dn" are
rows in [gui-design/DECISIONS.md](gui-design/DECISIONS.md). Visual design: the design pack,
[gui-design/SPEC.md](gui-design/SPEC.md) frames F1–F7.

## Goal

`rusty_term`'s window attaches to a rusty_mux session and draws what the server owns: sessions,
windows, each window's split tree of blocks, leases and block status (pack D28). Local mode stays
until C69 (auto-start, idle exit) and C45 land, then retires (ROADMAP question 1). C45 is merged;
C69 is arc 3 and has its own go, so it does not block this arc, only the retirement.

## Shape

The workspace layer check (`.github/scripts/check_workspace_layers.py`) forbids a libs crate from
linking an apps crate. `rusty_term` is libs and every `rusty_mux_*` crate is apps, so the mux side
can't live in `rusty_term`. Ports and adapters instead:

- **Port, in `rusty_term`:** a session source the window draws from: the layout tree, a byte
  stream and input sink per block, and lease and status events. Today's local panes become one
  adapter behind it, with no behaviour change.
- **`rusty_mux_client` (new lib, apps/rusty_mux):** the attach protocol now in
  `rusty_mux_tui`'s `main.rs` (connect, attach, take over, reach, welcome, upgrade), plus
  `checkpoint::Receiver` and `style::Style` moved out of the server, so no client links the
  server. The TUI switches to it.
- **`rusty_mux_gui` (new binary, apps/rusty_mux):** links `rusty_term` with `gui` and the client,
  and implements the port over the mux. This is a new rusty_mux → `rusty_term` edge (revises the
  rule that only the server and TUI link it).

Both crates and the new edge need Nano's go (owner gate). Alternatives considered: client code in
the TUI's lib (the GUI would pull in the server and TUI), or a copy inside the GUI crate (two
protocol implementations).

## Slices

Each slice is one PR. Every F-item's look goes through `rusty-term-design-expert`; visual rows use
gui-design D45–D49.

| # | Slice | Gate |
|---|---|---|
| S1 | Extract `rusty_mux_client`; the TUI uses it. No behaviour change | new crate (Nano) |
| S2 | Session-source port in `rusty_term`, local adapter; `rusty_mux_gui` attaches to one server window and draws its split tree | new crate and edge (Nano) |
| S3 | F1 chrome: top strip, block headers with C21 status, ribbon; F2 prefix hints; F7 zoom | arc GO covers C16, C21 |
| S4 | F4 session switcher and thumbnails, F6 detached block, F5 reconnecting (30 s grace, D44; leases survive a reconnect, D83) | arc GO |
| S5 | F3 take over: confirm first (gui-design D44); the client re-asks for the lease after (D46) | arc GO |
| Later | C55 runs and prompt marks, C53 images, C54 clipboard, C61–C64 | each needs Nano's own go (D36); C54 and C63 are security-relevant |
| End | Retire local mode | after C69 and C45 |

A new message takes the next free `Features` bit from `message.rs` on its PR and gets owner
review (D27).

## Coordination

The "Build the updated Rusty Term interface" thread owns slices 2–4 of the pack (states, reduced
motion, Instrument Sans) and gui-design D50 up. This arc owns pack slice 5 (mux frames).
