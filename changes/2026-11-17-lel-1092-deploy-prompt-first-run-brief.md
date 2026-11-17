---
category: Added
changelog: Let Em Loose #1092 paste-ready deploy prompt for bbp and the BBP directory, the first-run brief, and open question Q50 (docs only).
---
## 2026-11-17 - Let Em Loose deploy prompt, first-run brief and Q50

- **Added:** `docs/projects/let-em-loose/1092-deploy-prompt-bbp-and-board-dir.md`: what BaileyAi lacks for #1092 (install `bbp` and `rusty_orch`, create `/srv/lel/bbp` for `orch-runner` with a reset SELinux label), leaving `lel-codex` alone. Nothing is deployed.
- **Added:** `docs/projects/let-em-loose/1166-first-run-brief-ssh-launcher.md`: the rusty_mux SSH host launcher brief, saved as the intent for the first run.
- **Added:** D140 in `DECISIONS.md` (one paid lens run approved 2026-11-17, superseding D134 for that run only) and `handoff/OPEN_QUESTIONS.md` Q50 for the two asks still open.
- **Known limitations:** docs only. The Claude `no_overage` entry in `assertions.json` is still Nano's to write.
