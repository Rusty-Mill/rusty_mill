---
category: Added
changelog: Let Em Loose #1380 layered project workspace step design (shared read-only base, per-session alternates clone, critic views, cleanup and disk floor), draft for approval.
---
## 2026-11-28 - Let Em Loose #1380 layered workspace design (draft)

- **Added:** `docs/projects/let-em-loose/1380-layered-workspace-design.md`, filling `726-sandbox-design.md` §7 (D180) with the layout, base refresh, alternates clone with a sealed orch git dir, jail roots, cleanup, disk floor and a five-slice plan. Expert defaults D228–D231 and Q68. Docs only; nothing is built.
- **Known limitations:** the build-cache mechanism (B2: reflink-seeded `target/` vs D182's sccache) waits for Nano; BaileyAi's `/srv` free space and filesystem are unmeasured (Q68).
