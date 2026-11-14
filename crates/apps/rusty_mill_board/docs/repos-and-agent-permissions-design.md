# Design: repositories apart from projects, and agents that act like humans

- **Status:** Accepted (2026-11-14, Nano: projects may span repos; an agent cannot grant Admin; token minting and rotation stay HTTP-only). Not implemented yet.
- **Scope:** `rusty_mill_board` (settings, registry, MCP/API). `rusty_tick` is unchanged (see "Tick impact").
- **Builds on:** ADR-0003 (projects), ADR-0004 (settings), #1667/#1668 (registered agents), #1675 (per-agent role and projects), #1677 (nicknames), #1643 (project status).

## 1. What exists today (verified in code)

**Repositories and projects.** `projects.rs` holds `repo -> {project -> paths}`. A project belongs to exactly one repository, and its paths are in that repository. Project names are unique across repositories. Every repository also gets an *implicit project named like itself* that owns the paths no other project of it does. A card carries only `project`; its repository is looked up (`Projects::repo_of`). Status (`Active/Paused/Experiment/Archived`) is per project, in its own file. In tick a project is a list named like the project.

So the two are already nested, but not separate:
- A project cannot span repositories. A repository cannot be described without also creating a project of the same name, so `apprentice_ai`, `mediboard` and `rusty_meshed` are each *a repository, a project and a status* at once. This is the confusion behind "set them to Paused in Settings > Repositories": the UI shows repositories, the status is stored on the project.
- A repository carries no data of its own (no GitHub slug, no archived flag). `github.rs` and the runner learn the repository from config, not from the registry.

**Agents.** A connector sign-in is fixed to a role. `agent_register` gives one agent an id and key; its `Grant {role, projects}` can only narrow the connector's at registration. An admin can replace it later (`settings_registered_agent_set`). `acting()` turns that into an `Actor {caps, projects}` and `Board::apply` checks capabilities and project scope in one place. Nothing branches on "human" versus "agent": `human` and `builder` hold the same twelve capabilities today. The differences that make agents less capable than a person are all *surface*, not *capability*:

| Gap | Where |
|---|---|
| Every `settings_*` operation is HTTP-only, deliberately not an MCP tool ("a model should not mint credentials") | `settings.rs` header |
| A project's status, repository, paths and a role's capabilities can only be changed through those operations | `settings_project_status_set`, `settings_projects_set`, `settings_roles_set` |
| A registered agent cannot be widened except by a human in Settings (fine: this is the grant) | `settings_registered_agent_set` |
| `project_add` is the only settings-like MCP tool | `api.rs` |

So "agents can do anything a human can, if granted" mostly means *expose the Admin operations over MCP*, not new authority machinery.

## 2. Part A: repositories as their own entity

### Data model

Two registries, linked by an explicit edge.

```jsonc
// projects.json (version 2)
{
  "version": 2,
  "repos":    { "rusty_mill": {"github": "rusty-mill/rusty_mill"}, "pythrust": {} },
  "projects": {
    "rusty_term": { "rusty_mill": ["crates/libs/ui/rusty_term"] },
    "pythrust":   { "pythrust": [] },
    "tooling":    { "rusty_mill": [".github"], "pythrust": ["ci"] }   // spans two repos
  }
}
```

- `Repo`: name plus optional facts (`github`). No status. A repository is *served* or not.
- `Project`: name, status, and a map `repo -> paths`. An empty list means "the project's share of that repo is whatever no other project of it claims" (today's implicit project semantics, made explicit).
- Invariants, checked once in `Projects::parse`: names valid and unique per kind; every edge names a known repo; within one repo no two projects claim overlapping paths and at most one project has the empty claim (the *rest* owner); every repo has a rest owner (see migration).
- Cards stay `project`-only. `repo_of` becomes `repos_of`. A card in a multi-repo project gets an **optional** `repo` field (default: the project's `default_repo`, an explicit field that is required whenever a project has more than one edge, so nothing depends on map order; single-repo projects change nothing). It is set at `card_add`/`card_assign`, validated against the project's edges.

**As built (PR 1):** each repository's own project stays implicit (it is the rest owner, as today), so the file never lists it. A file is written as version 2 only when a project spans repositories or a repository has a `github` slug; otherwise it is written in the older form, so the previous binary can still read it on rollback. A project's value is `{"repos": {repo: [paths]}, "default_repo": ...}`, with `default_repo` required only when it spans several.

### Options

| | A1: leave the shape, fix the UI | **A2: edge model above (recommended)** | A3: tags only (repo = tick tag) |
|---|---|---|---|
| Project spans repos | no | yes | yes, but unenforced |
| Repo holds several projects | yes | yes | yes |
| Enforcement of paths per repo | as now | as now, per edge | none |
| Cost | tiny | one parse/serialise rewrite + accessors | small, but ADR-0003 rejected tags as projects: cannot be enforced |

A2 keeps `Board::apply` scope checks untouched (they key on the project name), so the blast radius is `projects.rs`, three accessors and the settings UI.

### Migration (no card rewrite)

1. Version-1 files (`{"repos":{repo:{project:[paths]}}}`, or the flat form) parse as today and are converted in memory: each v1 project becomes one edge; each repo's implicit project (named like the repo) becomes an explicit project with the empty claim. Project names, statuses and cards are untouched, so tick lists and `board:project` keep working.
2. The settings write path writes v2; v1 stays readable for one release. A `.v1` backup of the file is kept next to it (same convention as the other settings files).
3. No card migration: cards already store only the project name.

### Effect on each surface

- **`card_assign`**: unchanged signature; moving between projects of different repositories now just changes the card's edge. Adds optional `repo` for multi-repo projects. Still needs Triage and both projects in scope.
- **Settings > Repositories**: two panels. *Repositories* (add/remove, GitHub slug) and *Projects* (status, per-repo paths, add/remove repo edges). Status moves out of the repository row, which removes the apprentice_ai / mediboard / rusty_meshed ambiguity. Removing a repo that still has edges is refused; removing a project with cards is refused (as columns are today).
- **`board_projects`**: reports `{repos: {...}, projects: {name: {repos: {repo: paths}}}, status}`. The old `{repos: {repo: {project: paths}}}` shape is kept as an additive key for one release so existing readers (runner, prompts) keep working.
- **Roadmap and metrics**: both group by project today and keep doing so. They gain a `repo` filter only (grouping by repo is a read over the edges). Milestones are tick tasks and stay project-level.
- **Runner**: `--project` stays mandatory; for a multi-repo project it also takes `--repo`. `board_owns` takes an optional repo.
- **GitHub sync**: gets its repository from the registry's `github` field rather than separate config (follow-up, not in the first slice).

## 3. Part B: the grant decides, not the identity type

### Principle

`Actor.caps` and `Actor.projects` are the only authority. No code asks whether the caller is a person or an agent. The remaining differences are closed by exposing Admin operations as MCP tools and by guard rails that apply to **everyone**, human or not.

### Changes

1. **Expose `settings_*` as MCP tools**, each running the existing `Settings::admin` path: same `acting()`, same `Admin` check, same lock, same validation. No new authority path. A human token and an agent hit the identical code.
2. **Exceptions by *result*, not by caller**: operations that return a secret (`settings_agent_add`, `settings_agent_rotate`, minting) stay HTTP-only. A secret would land in a chat transcript. This is a property of the operation, so it also holds for a human using MCP.
3. **No self-elevation** (applies to all actors):
   - an actor cannot change its own grant or revoke itself; editing a role the actor holds counts as changing its own grant (`settings_roles_set` refuses it);
   - no actor may revoke or narrow the last human Admin;
   - anything it grants (`settings_registered_agent_set`, `settings_roles_set`, `agent_enrol`) must be a subset of its own capabilities and projects. `Admin` and `Enrol` stay human-only grants made in Settings: today minted agents can never hold them, and an Admin agent that could grant Admin could register a second agent, raise it, and have it change the first's grant or demote the human Admins.
4. **Audit**: every settings call is appended as a `board_events` record (actor id, nickname if any, operation, target, outcome; denied attempts included, so probing past a grant is visible; no values that could be secret). This covers the "settings change history unbuilt" gap and is what lets Nano see what an agent did with a grant.
5. **Grant flow**: agent registers (narrow by default) -> Nano raises its grant in Settings -> it acts. This already exists; the nickname from #1677 shows in the audit.
6. **Finer capabilities (not now)**: splitting `Admin` into `admin:agents`, `admin:projects`, `admin:roles` is the obvious next step, but there is no second call site today. Defer until a real grant wants a partial Admin (YAGNI). Roles stay admin-managed data and already allow this later without a schema change.

### Options

| | B1: keep HTTP-only | **B2: expose, with guard rails (recommended)** | B3: split Admin first |
|---|---|---|---|
| Agent can set project status, add repos | no | yes | yes |
| New authority code | none | none (reuses `admin`) + two guard rails | large |
| Risk | agents blocked | agent with Admin can reshape the board, bounded by 2, 3, 4 | same, plus migration |

## 4. Tick API impact

**None for the recommended design.** Tick sees only project = list name and the card's own tags. Repositories live in the board's registry, cards gain at most an optional `repo` field in the board's own record, and the tick tag set already carries it if we choose to mirror it later. So nothing needs deploying together with `rusty_tick`. The one optional item, mirroring `repo` as a tick tag for tick-side queries, is deferred and would need coordinating with the rusty_tick project; it is flagged here only so it is not forgotten.

## 5. Rollout (small PRs, each independently deployable, board only)

1. **Registry v2 + parser + migration** (`projects.rs`, tests for v1 round trips, overlaps, rest owner). No behaviour change.
2. **Accessors** (`repos_of`, `board_projects` additive shape, optional card `repo`, `card_assign`/`card_add` validation, `board_owns` repo arg).
3. **Settings UI**: Repositories and Projects panels; move status out of the repo row. The apprentice_ai / mediboard / rusty_meshed Paused change is made here (or earlier in the existing UI once #1685 is deployed; do not wait for this).
4. **Guard rails + audit events** (self-elevation, subset grants, `board_events` records).
5. **Expose `settings_*` over MCP** (except secret-returning ones).
6. Follow-ups, each on demand: roadmap/metrics repo filter, runner `--repo`, GitHub slug from registry, optional tick tag mirror.

PR 4 lands before 5 so no tool is exposed without its guard rails. Each PR gets a deploy prompt in `mill-board/` as before.

## 6. Open questions for Nano

1. Do you want a project to be able to span repositories (A2), or is "one project, one repository, repository is just a parent" (A1 plus a cleaner UI) enough? Recommendation: A2.
2. May an `Admin` agent grant `Admin` to another agent (3.3)? Recommendation: no, it stays a human step in Settings (a reviewer showed that allowing it defeats the self-elevation rule).
3. Is HTTP-only for token minting/rotation acceptable (3.2)? Recommendation: yes, since MCP would put the secret in the transcript.

## 7. Rejected

- **Agent-type flag** (`kind: agent|human`) with special rules: reintroduces the identity-based decision the task is removing.
- **Repository as a tick tag only**: not enforceable (ADR-0003).
- **A separate settings service**: no forcing function; the modular monolith already has one write path.
