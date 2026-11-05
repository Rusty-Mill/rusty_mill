# Arc 2: the GUI as a mux client (plan)

Roadmap arc 2, "One GUI for terminal and mux" ([ROADMAP.md](ROADMAP.md)). Owner GO: Nano,
2026-11-05 20:55Z, project thread message `cmsg_014f6o52E2iAAAgMm9v8hh1w3582EEHuK6PNVK64YWHa5q`
("1 go"), restated on this PR ([comment](https://github.com/Rusty-Mill/rusty_mill/pull/1529#issuecomment-6669837542),
"go on arc 2, two crates OK"), as D37 requires. Recorded in D95–D98.

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
  and implements the port over the mux. It is the one mux crate that links the `rusty_term`
  front-end (D96, superseding D22). Every other mux crate still links only `rusty_term_core`,
  without `l13`. This is not a layer exemption: an apps crate linking a libs crate is a downward
  edge the layer check allows, and `rusty_term` itself never links a mux crate.

Nano approved both crates and that edge (D96, restated on this PR in D98). Alternatives considered: client code in
the TUI's lib (the GUI would pull in the server and TUI), or a copy inside the GUI crate (two
protocol implementations).

## Slices

Each slice is one PR. Every F-item's look goes through `rusty-term-design-expert`; visual rows use
gui-design D45–D49.

| # | Slice | Gate |
|---|---|---|
| S1 | Extract `rusty_mux_client`; the TUI uses it. No behaviour change | approved (D96) |
| S2 | Session-source port in `rusty_term`, local adapter; `rusty_mux_gui` attaches to one server window and draws its split tree | approved (D96) |
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
