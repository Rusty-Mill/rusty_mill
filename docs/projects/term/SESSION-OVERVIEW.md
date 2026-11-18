# Session overview (card #1572)

Status: design, proposed; not authorized to build. Source: Nano, voice, 2026-11-18 (remind-me mem_757c3b59, card #1572 body and comment). Nano gates it.
Pack ids to reserve: handoff D336, Q95; gui-design D94, Q92 (re-check main on the day).

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
| agent, role/lens, card id, project per block | set at launch by the profile (LEL #1571) | **no** |
| card state, card to block link | Mill Board `card_execution_link`, held board-side | board only |
| waiting on human, progress line | agent self-report, display only, low trust | partial |
| other sessions (level 1) | the client has one control connection scoped to one session (handoff D109) | **no** |

## Wire (not allocated here)

Block metadata is new `Launch` fields plus a list/event field behind a new Features bit. Old peers see it, so it is an owner gate. Level 1 may need a server-wide session list. Free bits come from `proto/src/message.rs` only.

## Order

1. Block metadata on the wire and in `rusty-mux-ctl list` (cloud-buildable, owner gate).
2. TUI text overview (cloud-buildable).
3. GUI rows, tiles, zoom (needs a Windows GUI build; not cloud-buildable).

## Builds on

AGENT-HOSTING C46/C75-C78; D82, D87, D105-D107; gui-design D14, D44, D45-D49, D53-D58 (Let Em Loose tab; this view replaces the phase-lane drawing), D60, D61, D64. C21 run markers are not built: treat them as a prerequisite or fold them in.

## Open questions (proposed only; implementation gated until Nano's answer is recorded)

Every item below is a recommendation, not an authorized default. Nano gates this design (card tag `needs-nano`). Wire, security, privacy, retention and export behaviour must not be built until his answer is recorded in the pack by the expert.

- **Q95** How are agent, role, card and project set? a) new `Launch` fields plus a Features bit; b) packed into the label string; c) board-side only, the GUI asks orch. **Recommend a**: structured, and lease/lens display needs it. Owner gate on the wire change.
- Layout, tile look and zoom feel go to `rusty-term-design-expert` before any GUI slice.
