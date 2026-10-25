---
category: Changed
changelog: "Gap 5 decision S5 amended: the owner reviews the signing code (plan D4) before the native `rusty_tls` wiring merges."
---
## 2026-10-24 - Gap 5: S5 is an owner review

- **Changed:** S5 now names the owner as the reviewer of the native signing code, which is plan D4's "owner alone" option. It was "independent review". The amendment is recorded in ADR-0006, in `TLS-NATIVE-SIGNING.md` (sections 4 and 6 and the checkpoint), and in plan section 0 and D4. The original decision stays visible alongside the amendment.
- **Provenance:** the owner said "S5 is me" in the working session on 2026-10-24. This is the implementer's transcription, not yet restated from the owner's GitHub account.
- **Limits, stated in the records:** one reviewer, who owns the project and whose account authored commit `32db5c131` in #992. This is not an independent audit.
- **Unchanged:** a review recorded with reviewer, scope and exact commit is still required before the `rusty_tls` wiring merges. The reviewed code is on main at `698844212`. No code changes.
