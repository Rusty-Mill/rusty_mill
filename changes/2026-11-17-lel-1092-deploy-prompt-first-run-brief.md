---
category: Added
changelog: Let Em Loose #1092 paste-ready deploy prompt for bbp and the BBP directory, the second-run brief, and open question Q50 (docs only).
---
## 2026-11-17 - Let Em Loose deploy prompt, second-run brief and Q50

- **Added:** `docs/projects/let-em-loose/1092-deploy-prompt-bbp-and-board-dir.md`: what BaileyAi lacks for #1092 (install `bbp` and `rusty_orch`, create `/srv/lel/bbp` for `orch-runner` with a reset SELinux label), leaving `lel-codex` alone. Nothing is deployed.
- **Added:** `docs/projects/let-em-loose/1166-first-run-brief-ssh-launcher.md`: the rusty_mux SSH host launcher brief, saved as the intent for the second paid run (the first is the toy sweep smoke run).
- **Added:** D140 and D141 in `DECISIONS.md` (Nano approved two paid lens runs on 2026-11-17: the toy sweep smoke run, then the SSH launcher brief; codex paid now, claude after its assertion) and `handoff/OPEN_QUESTIONS.md` Q50, now resolved.
- **Added:** `docs/projects/let-em-loose/1166-smoke-run-prompt.md`: the paste-ready prompt for the local-only smoke run (no paid call), assuming the #1092 deploy. The deploy prompt now installs the lens configs from baileyrd/Factory (Factory#2 fixes the llama-swap address and model id).
- **Changed:** the #1092 deploy is now the idempotent script `docs/projects/let-em-loose/1092-deploy.sh` (fresh Factory clone, sha256 check of installed lens files, checks run as `orch-runner` from `/`), after the first BaileyAi run showed stray terminal characters in a pasted script, a leftover clone directory and a wrong `var_t` label expectation (D142: `/srv/lel/*` is `container_file_t` by policy).
- **Known limitations:** docs only. The Claude `no_overage` entry in `assertions.json` is still Nano's to write.
