---
category: Security
changelog: orch-bbp strips RUSTY_MUX_SOCKET from every spawned workload's environment.
---
## 2026-11-05 - orch-bbp: keep the mux socket out of workloads

- **Security:** `orch_bbp::SCRUB` now includes `RUSTY_MUX_SOCKET`, so an orch started inside a rusty_mux block never hands spawned agents, harnesses or `bbp` children a path to the mux control socket (Let Em Loose D13, D103 §4). First slice of the approved mux-hosted agent ptys design (D104, card #732).
- **Known limitations:** removes the variable only. The jail's mount namespace and seccomp `AF_UNIX` denial land with `lel-jail` in a later slice.
