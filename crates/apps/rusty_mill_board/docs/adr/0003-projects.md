# ADR-0003: Projects: one board, several applications

- **Status:** Accepted (2026-10-14)

## Context
Rusty Mill is a monorepo: one repository, one CI, one merge queue, one base branch, many applications under
`crates/apps/*`. super-board (the model for this board) separates work by giving each project its own GitHub
Project, config and base branch; that fits a repository per product, but here it would duplicate everything
shared and cut the links between cards of different applications.

## Decision
- **Repositories** group projects: `{"repos": {repo: {project: [paths]}}}` (a flat file is the repository
  `default`). Project names are unique across repositories, so a card carries only its `project` and the
  repository is looked up. Each repository has an implicit project named like itself that owns the paths no
  other project of it does. Paths are per repository. The UI's side panel lists repositories with counts and a
  project dropdown for the chosen one; it only filters.
- A card has a `project`. The registry (`--projects FILE`) maps each non-default project to the repository paths
  its work may change; `default` always exists and owns the rest. A card stored before projects existed reads
  as `default` (`#[serde(default)]`), so the record layout needs no migration.
- Scope is **enforced**, not advisory. A credential may be limited to projects. The check runs inside
  `Board::apply`, on the card being committed, so every write (move, claim, release, edit, comment, assign,
  take) is covered in one place and a card re-assigned between a caller's read and write is refused.
  `board_list` and `card_get` hide other projects from a limited credential.
- Moving a card between projects needs Triage and both projects in scope, and drops the claim.
- The runner serves one project (`--project`; mandatory on a multi-project board) and, after a stage
  succeeds, discards the whole run if it changed a path outside the project's paths or the shared ones.

## Consequences
- Open (unauthenticated) and shared-token boards trust the caller's claims, as before: scope is only as
  strong as the credentials. With `--identities` it is a property of the token.
- The sandbox's write scope is not narrowed to the project: builds need `target/` and caches in the worktree.
  Out-of-scope edits are caught at commit, not prevented while the command runs.
- A limited credential is told by `board_projects` only of its own projects (and the repositories holding them). It cannot rebuild the registry from that, so which paths a project owns is answered by `board_owns` from the whole registry: a runner limited to `default` still learns that a file belongs to a sibling, without being told the sibling's name or paths.
- `board_events` still reports every project's event metadata (ids, actor, kind, column); card text is not in it.
- GitHub sync does not know about projects.

## Alternatives rejected
- **A board per project** (super-board's model): see Context.
- **Tags as projects:** display-only, cannot be enforced.
- **Narrowing Landlock write roots to the project's directory:** breaks builds that write `target/` and caches.
