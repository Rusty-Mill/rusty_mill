---
category: Added
changelog: `bbp why --claim ID [--at REV]` replays one claim, naming the rule behind each change (#599).
---
## 2026-11-04 - Claim replay: `bbp why --claim`

- **Added:** `rusty_bbp::claim_history`, a pure fold that replays one claim's evidence and status changes and names the rule that fired: the accept or drop line with its numbers, a human decide, a `choose`, or the cause an escalation, reopen, void or supersede carries. No log change. `bbp why --claim ID [--at REV]` prints it as JSON (#599 decision replay; #722 reuses it).
- **Known limitations:** a `choose` that finds no other live candidate writes only the accept, so it reads as a decide.
