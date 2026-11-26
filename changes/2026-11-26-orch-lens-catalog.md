---
category: Added
changelog: rusty_orch lens catalog adapter for #723: reads Factory `domains.json`, the files it names, and the target repo's DOMAIN.md.
---
## 2026-11-26 - Lens catalog adapter (#723 slice 2)

- **Added:** `orch_lens::compose::parse_map` parses the domain map (`domains.json`), refusing unknown fields and any path that leaves the catalog. `rusty_orch` `lens::catalog::Catalog` reads the map, every file it names and the target repo's `DOMAIN.md` once at open (a missing named file fails at open), then composes per question through the pure composer. A missing map or pack means nothing is composed.
- **Known limitations:** `lens run` does not use it yet (slice 3). The map is JSON, like `lens.json`, rather than the design's `domains.toml`, so orch needs no TOML dependency. The Ai Env Manager manifest and the tool request for #1378 are not read yet: the manifest format is still undecided.
