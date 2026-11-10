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
  `rusty_mux_tui`'s `main.rs` (connect, attach, take over, reach, welcome). The checkpoint
  assembler and the cell style codec move out of the server into `rusty_mux_proto`, behind its
  `screen` feature, so no client links the server. The TUI switches to it. `upgrade` (SSH to
  UDP) stays in the TUI with the path code it drives.
- **`rusty_mux_gui` (new binary, apps/rusty_mux):** links `rusty_term` with `gui` and the client,
  and implements the port over the mux. It is the one mux crate that links the `rusty_term`
  front-end (D96, superseding D22). Every other mux crate still links only `rusty_term_core`,
  without `l13`. This is not a layer exemption: an apps crate linking a libs crate is a downward
  edge the layer check allows, and `rusty_term` itself never links a mux crate.

Nano approved both crates and that edge (D96, restated on this PR in D98). Alternatives considered: client code in
the TUI's lib (the GUI would pull in the server and TUI), or a copy inside the GUI crate (two
protocol implementations).

## Slices

Each slice is one PR (S2 is two, D103; S3 is three, D106; S4 is four, D108, D112). Every F-item's look goes through `rusty-term-design-expert`; visual rows use
gui-design D45–D49.

| # | Slice | Gate |
|---|---|---|
| S1 | Extract `rusty_mux_client`; the TUI uses it. No behaviour change | approved (D96) |
| S2a | Session-source port in `rusty_term` (`gui::source`), local adapter. No behaviour change | approved (D96; split D103, shape D101) |
| S2b | `rusty_mux_gui` attaches to one server window and draws its split tree (focus, lease, resize per D102); `rusty_term::gui::run_attached` | approved (D96) |
| S3a | Instrument Sans text path (gui-design D59) and the F1 top strip: session chip, a tab per window with its thumbnail, "+", clients (gui-design D45, D46; data D105) | arc GO |
| S3b | F1 block frames and headers: blocks laid out in pixels with 8 px gaps, each framed, headed by its folder and branch or its program and C21 status, and who else has input (gui-design D47, D48; data D105) | arc GO covers C21 |
| S3c | F1 ribbon; F2 prefix keys, hints and badges (D104); F7 zoom with the header button, Exit zoom and the rail (gui-design D49) | arc GO covers C16 |
| S4a | F4 session switcher under the session chip, switching on the one control connection (D109); running blocks lit in the strip's thumbnails (gui-design D60) | arc GO |
| S4b | F6 detached chip, popover and Reattach (gui-design D61) | arc GO |
| S4c | F5 reconnecting, error and reconnected banners (30 s grace, D44; leases survive a reconnect, D83; D110, D111, D113; gui-design D62) | arc GO |
| S4d | F5 "Session {name} ended" panel with Open switcher (D112; gui-design D62) | arc GO |
| S5 | Lease re-ask only: the GUI asks again for the focused block's lease after a reconnect or when `LeaseChanged` frees it (handoff D115). F3 take over (confirm first, gui-design D44) waits for C46/C77: it comes once agent blocks are known and taking over means asking for a grant (handoff D116) | arc GO |
| S6 | F3 take over (C46/C77 and lease requests merged, handoff D120): on an owner-only block, "Take over" confirms first (gui-design D44), asks the owner (`RequestLease`), takes the lease once granted, and "Give back" releases it; header and confirm per gui-design D64, behaviour per handoff D153 | Nano's go, 2026-11-09 |
| Later | C55 runs and prompt marks, C53 images, C54 clipboard, C61–C64 | each needs Nano's own go (D36); C54 and C63 are security-relevant |
| End | Retire local mode: R1 (#1569) makes `rusty_term --gui` exec `rusty-mux-gui`; R2 (card #1012) deletes `gui::run`, the window's local spawn path, `gui/control.rs` and `rusty_term ctl`, session files, the quake window and its hotkey, and `PaneIo::local`; R3 (card #1422) makes `source` required and drops the local chrome bar. The GUI is mux-client only | Done: R1 (#1569); R2 (#1588, handoff D158, scope D159); R3 (card #1422, Nano's go 2026-11-10, handoff D165). Local mode is fully retired |

A new message takes the next free `Features` bit from `message.rs` on its PR and gets owner
review (D27).

## Coordination

The "Build the updated Rusty Term interface" thread owns slices 2–4 of the pack (states, reduced
motion, Instrument Sans) and gui-design D50 up. This arc owns pack slice 5 (mux frames).
