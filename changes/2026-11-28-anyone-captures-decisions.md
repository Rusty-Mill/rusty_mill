---
category: Changed
changelog: Any agent may capture an owner decision in remind-me and tell the project expert; experts still own DECISIONS.md (Nano, 2026-11-28).
---
## 2026-11-28 - Anyone captures decisions; experts keep the log

- **Changed:** CLAUDE.md and the let-em-loose, rusty-term, millwright and rusty-fuse expert prompts no longer forbid callers from saving a remind-me decision. Any agent may capture one, then pass the memory id to the expert, which reuses it instead of adding a duplicate. Only the expert edits the pack's DECISIONS.md.
