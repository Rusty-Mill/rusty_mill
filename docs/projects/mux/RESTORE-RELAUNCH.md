# Relaunch a block's command on restore (card #1569)

Status: idea, parked (priority 0). Nano said "All" on 2026-11-18, so the note exists; building still needs his go. Source: remind-me has no entry for this card; the words are the card body. Pack ids to reserve: handoff D338, Q97.

## Today

C45 restores sessions, windows, layout and scrollback after a restart. D72/D92 [stated, Nano 2026-11-04]: a shell is restarted fresh; an explicit command comes back dead and read-only (C78). Reason: no repeated side effects. Store: D73-D76 (C45 store, RTCP screens as files, C70 spool, unencrypted state dir D74).

## Idea

Per block `restore: relaunch | shell | none`, default `shell` (today). `relaunch` re-runs the recorded command and writes a header line "relaunched after restore at TIME". Not CRIU: no process checkpointing.

## Proposed answers

- **Command source**: the `Launch` the server already stored for the block (C75/D76), not /proc (racy, nothing on Windows, D188/Q42) and not OSC 133, which marks commands inside a shell, not the launch. For a block that is just a shell, relaunch does nothing extra.
- **Safety**: deny by default. Only blocks explicitly set to `relaunch` run; an allowlist of argv can narrow it further. Never relaunch interactive one-shots by inference.
- **Store**: a `restore` field on the persisted block record (schema change, D73/D76) and on `Launch` (wire, owner gate). C70 and the RTCP checkpoint are unchanged: only the launch record is read.
- Names `relaunch|shell|none` are implementation detail.

## Open question

- **Q97** Does this override D72 for opt-in blocks? a) yes, opt-in only, allowlisted; b) no: keep D72 and add a one-key "re-run" in the held block. **Recommend b**: same user benefit, no side effects on restore, no store change. Needs Nano; a reverses D72 partly.

## Order if built

Cloud-buildable end to end (store, server, ctl, TUI header); the GUI only shows a header line.
