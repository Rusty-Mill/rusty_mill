---
category: Changed
changelog: Let Em Loose designs now live in `docs/projects/let-em-loose/`, committed in their first draft PR; #599 track records score pivotal reviews against ground truth only (D93, D94).
---
## 2026-11-04 - Let Em Loose: designs in the repo, pivotal track-record rule

- **Changed:** every Let Em Loose design is written to `docs/projects/let-em-loose/` and committed in its first draft PR, never only to `/mnt/project-files/` or session scratch, where the #594 and #599 designs were lost (D94). The let-em-loose-expert pack and the scoped `CLAUDE.md` rule say so.
- **Changed:** Nano's answer to the #599 track-record question is recorded (D93). Pivotal reviews are not scored against the board outcome. They are scored against ground truth where it exists, and otherwise left unscored and counted. The calibration report shows the pivotal counts per lens.
