# Relaunch a block's command on restore (card #1569)

Status: design; one-key re-run accepted (handoff D338, Nano 2026-11-18); relaunch on restore rejected. Source: the card body (remind-me has no entry for this card).

## Today

C45 restores sessions, windows, layout and scrollback after a restart. D72/D92 [stated, Nano 2026-11-04]: a shell is restarted fresh; an explicit command comes back dead and read-only (C78). Reason: no repeated side effects. Store: D73-D76 (C45 store, RTCP screens as files, C70 spool, unencrypted state dir D74).

## Decided (D338, Nano 2026-11-18)

D72 stands: restore never runs anything on its own (a shell restarts fresh; an explicit command comes back dead and read-only). Instead, a held block gets a one-key "re-run".

- **Command source**: the `Launch` the server already stored for the block (C75/D76), not /proc (racy, nothing on Windows, D188/Q42) and not OSC 133.
- **Agent blocks** re-run through the orchestrator and the profile, not by replaying argv: their tokens are scoped and short-lived.
- **No store change**: no `restore` field, no schema change; the re-run is a user action in the held block, with a "re-run at TIME" header line.
- Key and names are implementation detail.

## Rejected

Opt-in `relaunch` on restore with an argv allowlist (would partly reverse D72).

## Order

Cloud-buildable (server, ctl, TUI header); the GUI only shows a header line. Agent re-run needs the orchestrator side.
