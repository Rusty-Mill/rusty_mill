---
category: Added
changelog: orch-lens `compose`, the pure #723 lens composer (lens + DOMAIN.md + domain-selected knowledge and standards).
---
## 2026-11-26 - Lens composition core (#723 slice 1)

- **Added:** `orch_lens::compose`: `select` maps a question's claim domains to knowledge and standards files through a domain map; `compose` appends `# Project` (DOMAIN.md), `# Knowledge` and `# Standards` to a lens prompt, deduped and sorted so domain order never changes the bytes. With nothing selected and no domain pack it returns the lens prompt unchanged. Oversized files are dropped; over budget, knowledge goes first, then the domain pack, then standards, and every drop is reported. Pure, no I/O; nothing calls it yet.
- **Known limitations:** the Factory catalog adapter (slice 2) and the `lens run` switch (slice 3) are not built. The tool request for #1378 lands with the manifest reader in slice 2.
