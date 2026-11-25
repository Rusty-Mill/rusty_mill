---
category: Changed
changelog: Let Em Loose #447: record Nano's answers on BBP human gates (D190, D191) and its implementation defaults (D192-D194).
---
## 2026-11-25 - Let Em Loose #447: BBP human-gate decisions recorded

- **Changed:** `docs/projects/let-em-loose/DECISIONS.md` adds D190 (no claims-mode `bbp human` verb needs a signed receipt for now) and D191 (the ask-first BBP/host items of the #447 step 6 design approved as a set: `task_opened.gates`, `bbp open --gates scratch`, `bbp human --approval`, the `--op by:` refusal). Source: Nano, project thread message `cmsg_014AqbZs58L565hMV2pFkCX5CZatgThTCScc6TZfje7dmZ`, 2026-11-25.
- **Changed:** adds [default]s D192 (a receipted answer is op `by:<approver>\/rcpt:<sha256>`, exclusive with `--op` and `--decision`), D193 (on an owned task only `approve-plan` and `approve-merge` need a receipt) and D194 (the board public key is committed at `crates/apps/rusty_bbp_host/keys/board-approval.ed25519.pub` and compiled in).
