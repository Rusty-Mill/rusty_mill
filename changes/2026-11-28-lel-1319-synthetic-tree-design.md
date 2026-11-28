---
category: Added
changelog: Let Em Loose #1319 design for the 500 GB synthetic test tree, reference hardware and perf protocol, plus a read-only drive-stats script (docs only).
---
## 2026-11-28 - LEL #1319 synthetic test tree design

- **Added:** `docs/projects/let-em-loose/1319-synthetic-tree-design.md`: the seeded `lel_treegen` generator (pure plan, thin writer, manifest kept outside the tree), duplicate, decoy, NAS, link and edge-file mix, the perf measurement protocol and the reference-hardware options. `1319-drive-stats.ps1` prints aggregate shape stats of Nano's drives, no names, paths or hashes. Decisions D228 to D231, all `[default]`.
- **Known limitations:** Design only, awaiting Nano's approval; the reference machine is still his call. The PowerShell script has not been run (no PowerShell in the cloud session).
