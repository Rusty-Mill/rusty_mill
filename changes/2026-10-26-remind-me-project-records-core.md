---
category: Added
changelog: remind-me records projects as typed entities with evidenced stage, status and anchor claims.
---
## 2026-10-26 - Project records: the core module (ADR-0027 slice 1)

- **Added:** `remind_me_core::project`. A project is an entity with a live `project.stage` claim; stage, status and the repo, path, tracker and parent anchors are SPO fact memories under the closed `project.*` predicate set, each citing at least one evidence memory. `list`, `get` and `history` project the claim chain into a typed `Project`; `set` and `revert` are the only writers. A claim is never edited: undoing one writes a compensating claim that restores the earlier value, cites its evidence plus a `because` memory, and names what it reverts.
- **Added:** `Memories::with_predicate_prefix`, the claim-chain read.
- **Changed:** `remind_me_add`, `remind_me_update`, `remind_me_annotate` and `remind_me_decompose` refuse `project.*` predicates and edits to claim memories, so project state has one writer. Delete, bulk delete and bulk tag refuse claim memories (a batch containing one changes nothing), and consolidation never merges one.
- **Security:** a `because` sentence and a claim's content go through the same secret scrubber as any capture; a project or parent name that looks like a secret is refused.
- **Added:** `set` and `revert` write in one transaction; claim values are revalidated before any write; equality is per predicate, so paths compare case-sensitively; claims with no subject or a miscased `project.*` predicate are reported in `ProjectList::invalid`.
- **Known limitations:** no tool, HTTP route or dashboard view yet, and no proposals or backfill; those are slices 2 and 3. A malformed claim arriving by sync is reported in `ProjectList::invalid` rather than repaired.
