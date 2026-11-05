---
category: Added
changelog: orch-lens alien module, the pure core of the #708 alien review lens (trigger, suspects, ranking, probes).
---
## 2026-11-04 - Alien review lens core

- **Added:** `orch_lens::alien` (#708, design D75), pure and shadow-only. `eligible` runs a hunt on a Rust candidate once reviews settle. It accepts irreversible claims, and reversible ones only at #596's value threshold or in a deterministic 1-in-10 control sample. With no threshold frozen yet, only irreversible claims qualify. `parse_suspects` and `parse_probe` read the enumerate and hunt replies strictly. Unknown mechanisms are dropped, and sites must be a `module::fn` or a path. `rank` keeps the top 3 by unavailability against the card's evidence notes, then coherence, then mechanism and site. `RUST` is the approved 12-entry defect-mechanism list.
- **Known limitations:** nothing calls it yet. Jobs, probe runs, orch-store rows and the `--alien` shell come in later PRs.
