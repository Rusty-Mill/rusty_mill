# ADR-0083: Read `Bridge.dll` to learn the game's socket format

- Status: Accepted
- Date: 2026-10-11
- Deciders: baileyrd (owner)
- Related: `ARCHITECTURE.md` Non-goals, `docs/research/RLBOT-BRIDGE-DLL.md`, `rb_rl_game_bridge`
- Supersedes/Superseded by: amends the "no decompilation" non-goal for this one purpose; supersedes nothing

## Context

`rb_rl_game_bridge` has every part of RLBot's game-facing half except the byte format of the
socket Rocket League dials (`RLBot_ControllerURL`). That format is defined only inside
`Bridge.dll`, which RLBot ships closed-source "due to legal reasons". The project's non-goal
says it does not copy or decompile Psyonix's code; `Bridge.dll` is RLBot's, not Psyonix's, but
the spirit (do not reproduce a vendor's closed implementation) applies.

## Decision drivers

- The owner wants a working game link for repeatable bot runs against the real game.
- Interoperating with a socket protocol is different from copying an implementation.
- The legal posture of the DLL is the maintainers' and is unknown to us.

## Considered options

1. Stay with `SimGameLink` only.
2. Capture the game's traffic on a loopback socket the owner controls.
3. Read the DLL's metadata and IL to recover the wire format.

## Decision

Options 2 and 3 are allowed, **for the wire format only** (framing, message ids, field
encodings). The owner approved this on 2026-10-11.

Boundaries that stay:

- Nothing from the DLL is copied: no IL, no decompiled source, no code translated line by
  line. What is written down is the *format* (a table of messages and fields) in prose and
  tests, and the Rust implementation is written from that table.
- The DLL is not redistributed and not committed; decompiler output is not committed.
- Psyonix's game code and assets stay out of scope as before.
- If the format cannot be recovered without reproducing the DLL's logic, we stop and ask.

## Consequences

### Positive

- A real-game `GameLink` becomes possible.

### Negative / tradeoffs

- The legal question is the owner's risk, not settled here. The maintainers may object.
- The format may change between `Bridge.dll` versions (v0.9.2 today); the spec records the
  version it was read from.

## Validation and revisit triggers

- A `GameLink` decoded against a recorded capture from the real game.
- Revisit if RLBot publishes the format, or if the maintainers object.
