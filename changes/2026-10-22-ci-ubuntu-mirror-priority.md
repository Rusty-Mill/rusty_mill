---
category: Fixed
changelog: Prefer the existing official Ubuntu archive over Azure during GitHub-hosted Linux environment setup.
---
## 2026-10-22 - Prefer the official runner Ubuntu mirror

- **Fixed:** before APT update, swap only recognized Azure/archive mirror priorities on GitHub-hosted Linux runners. Preserve other mirrors, source suites, signature verification, required packages, checks and the 15-minute setup cap; absent or unexpected layouts are left unchanged with a warning.
- **Known limitations:** fixture tests establish configuration behavior, not download performance. An unchanged-package Linux CI setup must reach compilation within the existing cap before throughput improvement is established.
