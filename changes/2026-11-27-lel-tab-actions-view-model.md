---
category: Added
changelog: Let Em Loose tab Approve, Reject and Choose view model in rusty_term (#1416 slice 4, not yet drawn).
---
## 2026-11-27 - LEL tab decision actions

- **Added:** `gui/lel_act.rs` builds `inbox_act` arguments (displayed rev as the fence, dry run first), validates Reject and Choose against the row, reads a stale-rev refusal as "re-read, do not resend", and holds the D58 confirm (Cancel focus first) and the side-by-side candidates. No UI, no I/O, no board change.
- **Known limitations:** Nothing sends the calls yet. The claims, intent and spend read endpoints (D173) are a separate PR; slice 4 needs only `inbox_list` and `inbox_act`.
