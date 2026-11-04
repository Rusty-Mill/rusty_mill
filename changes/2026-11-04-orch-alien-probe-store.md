---
category: Added
changelog: Alien lens hunt and probe rows in orch-store and `rusty_orch lens alien report` (#708).
---
## 2026-11-04 - Alien lens hunt and probe rows and report

- **Added:** `orch_lens::alien` classifies a probe from its runs on the candidate and on base (design D75 §6). A probe that fails to build on the candidate is `error`. One that passes is `pass`. One that fails on both candidate and base is `pre-existing`. One that passes on base and fails on the candidate is a `regression`, the only kind that counts as a finding. One that fails on the candidate and does not build on base is `new-code-fail`. A candidate failure with no base run is refused rather than classified.
- **Added:** `HuntRow` records each hunt, including hunts that produced no probe, with its tokens and cost. `summarize` counts every hunt once, from hunt rows and probe rows together, and gives the §8 shadow numbers. `judge` decides the success bar over a frozen cohort, the first 20 hunts by start time, so later hunts never change it. It checks both halves: one novel regression in that cohort, and lens spend under 15% of the claims' spend. Claim spend is not tracked yet, so a met finding rate reports the spend half as unassessed and the bar stays unmet.
- **Added:** orch-store keeps hunt rows in `hunts.mmap` (`HuntRecord::v1`) and probe rows in `probes.mmap` (`ProbeRecord::v1`) through `Store::save_hunt`, `Store::hunts`, `Store::save_probe` and `Store::probes`. `rusty_orch lens alien report --state <dir> [--task <t>]` prints the probes with the totals and the bar.
- **Known limitations:** nothing writes hunt or probe rows yet. Hunts and sandboxed runs come with the `--alien` shell slice.
