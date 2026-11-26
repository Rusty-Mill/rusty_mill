---
category: Security
changelog: BBP human gates refuse self-approval (#447 step 6) - an owned task's plan and merge approvals need a receipt Mill Board signs, scratch tasks are limited to repos the operator lists, and `bbp metrics` verifies each receipt again.
---
## 2026-11-25 - BBP owned gates and board receipts (#447)

- **Added:** `task_opened.gates` (`scratch`; absent means owned and isn't written, so owned logs keep their bytes). The engine refuses scratch gates with claims mode (D190, D191).
- **Added:** `bbp human --approval FILE`. On an owned task, `approve-plan` and `approve-merge` need a board-signed Ed25519 receipt bound to the task, gate, artifact, run and fence. The receipt is stored as a blob, and the answer runs under `by:<approver>/rcpt:<sha256>` (D192, D193). The board key lives at `crates/apps/rusty_bbp_host/keys/board-approval.ed25519.pub` and is compiled in (D194).
- **Added:** `bbp open --gates scratch`, refused unless the repo is on `<dir>/gates/scratch-repos.log`. Only `bbp gates scratch-add --source REF` writes that file (D195).
- **Added:** Mill Board `approval_issue` signs receipts with `--approval-key FILE`, an owner-only seed created on first start, whose public key is logged. Only a token credential of the caller's own can get one, since it names the approver; a named actor and role or a claude.ai connector cannot. A plan needs `approve`; a merge needs `admin`, and any admin counts as the owner. Every issued receipt is recorded in `FILE.issued.jsonl`. A shared fixture pins the receipt format on both sides.
- **Added:** Orch `Config.gates` passes `--gates scratch` at open.
- **Changed:** `bbp human --op` may not carry a `by:` label. `bbp metrics` now reports owned approvals without a verifying receipt as `self_approved`, and adds `scratch`. Host and Orch e2e fixtures open scratch tasks.
- **Known limitations:** the committed key file is empty until Nano starts the board with `--approval-key` and commits its public key, so until then every owned approval is refused. Orch's gate relay and the board's inbox don't pass receipts yet: an Orch-driven owned task's approvals are refused, which fails closed. No owned tasks are live today.
