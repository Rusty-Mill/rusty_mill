---
category: Changed
changelog: orch-lens questions carry a reply shape (board posts or raw text), the first #708 alien review lens slice.
---
## 2026-11-04 - Lens questions carry a reply shape

- **Changed:** `orch_lens::Question` takes `shape: Shape` (`Posts(Allowed)` or `Raw`) instead of `allowed`, and `Answer` carries `reply: Reply`. Every adapter (claude, codex, openai) checks replies with the new `orch_lens::check`, so the #708 alien lens's enumerate and hunt replies, which post nothing, can pass the port. The runner still asks only `Posts` questions; a `Raw` reply to one fails the job as `Permanent`.
- **Added:** the approved #708 design, `docs/projects/let-em-loose/708-alien-lens-design.md` (D68), with D67 unparking #708.
- **Known limitations:** nothing uses `Shape::Raw` yet; the alien lens core follows in its own PR.
