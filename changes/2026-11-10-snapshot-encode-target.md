---
category: Changed
changelog: mux ROADMAP Phase 0's snapshot-encode target (≤ 11.2 ms) is marked met, with Beast measurements.
---
## 2026-11-10 - Snapshot encode target met

- **Changed:** mux ROADMAP Phase 0 and CAPABILITIES now record that snapshot encode meets the ≤ 11.2 ms target reliably (card #786). On Beast at main `db325fada`, in release, the RTCP v6 checkpoint for 200x50 + 10k lines encodes in 1.51 ms best and 3.6 ms worst over 147 runs (403 KB). The repo's `measure_checkpoint_encode` agrees at 1.52 ms best. Docs only.
- **Known limitations:** no CI instruction gate covers encode, so there is no IR ceiling to ratchet; `measure_checkpoint_encode` is the only bench. Profiled top costs, kept for later: `run_len`'s per-row run scan 56%, run text and char decoding 11%, style refs and varints 11%.
