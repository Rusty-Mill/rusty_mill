---
category: Added
changelog: Let Em Loose #1092 paste-ready deploy prompt for bbp and the BBP directory, the second-run brief, and open question Q50 (docs only).
---
## 2026-11-17 - Let Em Loose deploy prompt, second-run brief and Q50

- **Added:** `docs/projects/let-em-loose/1092-deploy-prompt-bbp-and-board-dir.md`: what BaileyAi lacks for #1092 (install `bbp` and `rusty_orch`, create `/srv/lel/bbp` for `orch-runner` with a reset SELinux label), leaving `lel-codex` alone. Nothing is deployed.
- **Added:** `docs/projects/let-em-loose/1166-first-run-brief-ssh-launcher.md`: the rusty_mux SSH host launcher brief, saved as the intent for the second paid run (the first is the toy sweep smoke run).
- **Added:** D140 and D141 in `DECISIONS.md` (Nano approved two paid lens runs on 2026-11-17: the toy sweep smoke run, then the SSH launcher brief; codex paid now, claude after its assertion) and `handoff/OPEN_QUESTIONS.md` Q50, now resolved.
- **Known limitations:** docs only. The Claude `no_overage` entry in `assertions.json` is still Nano's to write.
