---
category: Added
changelog: rusty-mux-gui can take over an agent's block. It asks you to confirm, asks the block's owner, and lets you type once the owner agrees; "Give back" hands the input back.
---
## 2026-11-09 - rusty-mux-gui: take over an owner-only block

- **Added:** F3 "Take over" (arc 2, `docs/projects/term/ARC2-MUX-CLIENT.md` S6; gui-design D19, D44, D64; handoff D120, D153). An owner-only block's header shows "Agent has input" and "Take over". The button focuses the block and opens a confirm, "Take input from {program}?". Confirming asks the owner (`RequestLease`), and the header shows "Asking {owner}…" with "Cancel ask". Once the owner grants, the pane takes the lease, and the header shows "You have input" with "Give back". The header also says when the owner can't be reached, when the ask is refused, and when the grant ends.
- **Added:** the GUI's block connections also negotiate `INPUT_GATE` and `LEASE_REQUESTS` (`rusty_mux_gui::source::BLOCK`), and follow `GrantChanged` for their own client. On an owner-only block the lease is asked for only after a confirmed take-over and a grant. After Give back it isn't asked for again until the next take-over. A grant that arrives after Cancel ask is not used.
- **Changed:** `rusty_mux_client::attach` gains `reach_with`, and `take_over_with` takes the block connections' features, so a client that asked for more than `PANE` keeps it across a reconnect. The TUI is unchanged. rusty_term's session-source port gains `Feed::Take`, `Take`, `Refusal`, `Gate`, `BlockInfo::gate`, and `Act::{TakeOver, CancelAsk, GiveBack}`.
- **Known limitations:** a block whose owner is gone for good shows "{program} has no owner now" only after the next List (there is no event for it). Re-taking on a live grant shows "Asking {owner}…" for a moment while the lease is on its way.
