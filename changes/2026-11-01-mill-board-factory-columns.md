---
category: Added
changelog: Mill Board ships the software factory's workflow (Planning, Architecting, Building, V&Ving) with admin-only gates, plan/architect/test capabilities and a column retag tool with rollback.
---
## 2026-11-01 - Mill Board: the factory's columns

- **Added:** `deploy/factory.workflow.json`, the factory's stages as a board workflow: Backlog, Ready, Planning,
  Architecting, Building, V&Ving, Done, Blocked. The human gates (and every move out of Blocked) need `admin`;
  with movement rules off, a move that jumps over a gate still does. Each work column names who `card_take` serves from it, and the roadmap's Now lane is the four work stages.
- **Added:** capabilities `plan` and `architect`, and default roles `planner`, `architect` and `tester`. A move
  may be listed once per capability, so V&Ving to Done is a gated `merge` or a `close` recording a forge merge.
- **Added:** `--retag-columns FROM=TO,...` (dry run unless `--apply --retag-record FILE`) and `--undo-retag FILE`
  re-file cards when columns are dropped. The plan, with each card's revision, is saved before any write; undo
  restores only cards nobody has changed since. Retags keep the claim.
- **Changed:** capability `qa` is now `test`; stored roles and requests saying `qa` still read as `test`. The
  default `reviewer` role no longer holds `merge`, which is a human gate.
- **Known limitations:** nothing changes on a running board until an admin installs the workflow; drop columns
  only after retagging their cards, since a card whose column the workflow lacks reads as backlog.
