---
category: Added
changelog: rusty_agent_gateway gains the core of its skill library (Let Em Loose #1381): a checked skill-pack catalog with per-pack digests, and strict `[skills].packs` in lens policies. Nothing is served yet.
---
## 2026-11-28 - Gateway: skill pack catalog core

- **Added:** `agentgateway-mcp` `SkillCatalog`, built once from `(pack, path, bytes)`. It lists and reads `skill://<pack>/<path>` for the packs a caller may see, and gives each pack a sha256 digest of its paths and contents.
- **Added:** `LensPolicies::skill_packs` returns the packs a token's verified lens policy names in `[skills].packs`, and none for an unknown lens or a digest mismatch.
- **Changed:** `[skills]` in a lens policy is no longer opaque. It takes only `packs`, a list of non-empty strings; any other key refuses the policy.
- **Security:** pack names and path segments must be `[A-Za-z0-9._-]+` and not `.` or `..`. A file must be UTF-8 and at most 64 KiB. Any bad file refuses the whole catalog. A read outside the caller's packs looks the same as a missing file.
- **Known limitations:** not wired into the federation or config yet; the loader waits on two owner questions (`docs/projects/let-em-loose/1381-skill-libraries.md`).
