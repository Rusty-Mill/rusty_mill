# Session overview (card #1572)

Status: design; launch fields accepted (handoff D336, Nano 2026-11-18). The wire slice still passes the owner gate when its PR is opened. Source: Nano, voice, 2026-11-18 (remind-me mem_757c3b59, card #1572).

## What

One mux session per project; agents run in blocks inside it. The overview answers: in this project, what is worked, by which agent, in which role, right now.

Three zoom levels, each one click (or one key) away both ways:

1. **Mission command**: one tile per session. Agents running, cards active, anything waiting on a human. This is the S4 switcher ([gui-design](gui-design/README.md) D60) plus per-session agent counts.
2. **Session overview** (this note): one row per active card, the blocks working it as tiles beneath. Non-agent blocks (own shells) in a last row.
3. **Block**: the live terminal.

Keyboard: palette (C08) commands plus a prefix chord for up and down. Chord letters are an implementation default.

## Rows and tiles

- Row = card. Header: card id, title, state ribbon (ready, building, verifying, done). The ribbon describes the work, not the workers: no lanes, no handoff columns.
- Tile = block: agent identity, lens/role, state (running, waiting on human, held after exit, failed), lease holder, live thumbnail, elapsed. Click opens the block.
- Held and finished blocks stay until dismissed. Archived ones are reached from the card row ([AGENT-ARCHIVE](../mux/AGENT-ARCHIVE.md)).
- TUI: the same tree as text, one line per tile.

## Data

| Need | Source | Exists |
|---|---|---|
| state, lease holder | C20 OSC 7501 status, C74 hook status (`AGENT_STATUS`, bit 33), `Entry::Block.input` (D64) | yes |
| held/failed | C78 hold on exit, exit code | yes |
| thumbnail | S4 switcher | yes |
| agent, role/lens, card id per block | set at launch by the profile (LEL #1571); no `project` field, the session is the project (D336) | **no** |
| card state, card to block link | Mill Board `card_execution_link`, held board-side | board only |
| waiting on human, progress line | agent self-report, display only, low trust | partial |
| other sessions (level 1) | one `List` covers every session and its windows (handoff D109, `state.rs` `list`), already used by the GUI switcher | yes; only the per-session agent counts are missing (they follow from the block metadata) |

## Wire (D336)

Block metadata is new `Launch` fields `agent`, `role`, `card` (no `project`) plus a list/event field behind a new Features bit (next free on main). The server stores and echoes them and never branches on them. Old peers see it, so it is an owner gate. Level 1 needs no new list call: session discovery exists (D109); only agent counts per session are new, derived from that metadata. Free bits come from `proto/src/message.rs` only.

## Order

1. Block metadata on the wire and in `rusty-mux-ctl list` (cloud-buildable, owner gate).
2. TUI text overview (cloud-buildable).
3. GUI rows, tiles, zoom (needs a Windows GUI build; not cloud-buildable).

## Builds on

AGENT-HOSTING C46/C75-C78; D82, D87, D105-D107; gui-design D14, D44, D45-D49, D53-D58 (Let Em Loose tab; this view replaces the phase-lane drawing), D60, D61, D64. C21 run markers are not built: treat them as a prerequisite or fold them in.

## Decided

- **Q95** (D336, Nano 2026-11-18): new `Launch` fields `agent`, `role`, `card` plus a Features bit; `project` dropped; the server stores and echoes, never branches.
- Layout, tile look and zoom feel go to `rusty-term-design-expert` before any GUI slice.
