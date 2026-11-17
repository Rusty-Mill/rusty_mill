---
category: Added
changelog: Let Em Loose #1092 deploy layout design for BaileyAi (docs only, awaiting approval).
---
## 2026-11-16 - Let Em Loose #1092 deploy layout design

- **Added:** `docs/projects/let-em-loose/1092-deploy-layout.md`: binaries, users, directories, systemd, network, upgrades and verification for the BaileyAi deploy (D131), with Mill Board as its own `mill-board` user reaching `bbp` through a narrow sudo rule (D132). Docs only; nothing is deployed.
- **Known limitations:** the llama-swap port on BaileyAi is unverified.
