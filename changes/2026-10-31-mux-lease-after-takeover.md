---
category: Fixed
changelog: rusty-mux asks for its block's input lease again after a takeover, so typing still works after a long UDP outage followed by a fallback to SSH.
---
## 2026-10-31 - rusty_mux lease after a takeover

- **Fixed:** keys were dropped after a UDP path went silent for 30 s and the client then fell back to SSH (C13, found by the mux-expert review of #1285).
  - The server releases the lease of a silent association's clients, and a takeover on another connection does not give it back, by design (DECISIONS.md, UDP lease after silence).
  - The client assumed its takeover still held the lease, so the server refused its keys until the next resize or focus change.
  - The client now asks for the focused block's lease again on every takeover. Keys typed until the answer wait for it, and are dropped if another client took the lease meanwhile.
  - A taken-over block that is no longer in focus gives back a lease the server kept through a short loss. The server refuses the release unless this client holds the lease, so another holder keeps it.
- **Tests:** an idle but working UDP path is not lost after the 6 s outage window (#1288 coverage gap).
