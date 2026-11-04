---
category: Added
changelog: Alien lens probe rows in orch-store and `rusty_orch lens alien report` (#708).
---
## 2026-11-04 - Alien lens probe rows and report

- **Added:** `orch_lens::alien` classifies a probe from its runs on the candidate and on base (design D75 §6). A probe that fails to build on the candidate is `error`. One that passes is `pass`. One that fails on both candidate and base is `pre-existing`. One that passes on base and fails on the candidate is a `regression`, the only kind that counts as a finding. One that fails on the candidate and does not build on base is `new-code-fail`. `ProbeRow` and `summarize` give the §8 shadow numbers. `Summary::bar` judges the success bar of one novel regression per 20 hunts, and only once 20 hunts exist.
- **Added:** orch-store keeps probe rows in `probes.mmap` (`ProbeRecord::v1`) through `Store::save_probe` and `Store::probes`. `rusty_orch lens alien report --state <dir> [--task <t>]` prints them with the totals and the bar.
- **Known limitations:** nothing writes probe rows yet. Hunts and sandboxed runs come with the `--alien` shell slice.
