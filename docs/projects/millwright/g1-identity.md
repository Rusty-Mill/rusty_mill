# G1: Claude's GitHub identity and cross-family review

Report for rusty_mill card #24 (G1, O2). It proposes; it changes nothing. Nano changes
GitHub settings himself (card #24; ask-first list). Written 2026-11-12.

## What is true today

- **Cloud Claude acts as Nano.** Claude Code on the web and claude.ai reach GitHub through
  Nano's connected account. `get_me` in this session returns `baileyrd`, and Claude's
  reviews on PR #1643 are recorded as `baileyrd`, `author_association: MEMBER`. GitHub
  cannot tell those reviews from Nano's.
- **Dottius has its own account** (`dottiusmaximus`), so its reviews are already distinct.
- **The main ruleset** (id 24802062, as recorded on #24 on 2026-10-15; Nano to confirm it
  is unchanged): pull request required, 0 approving reviews, no code-owner review,
  `required-gate` check, merge queue (merge commit), no deletion or force push, no bypass
  actors.
- **CODEOWNERS** is review routing only: `*` and the high-impact paths go to `@baileyrd`.
- **Mill Board** (live, `board_roles`): `builder` holds all ten capabilities including
  admin, merge and triage; `reviewer` holds review and merge. Agents sign in as `admin`.
  That half is M1 and is fixed by board configuration, not GitHub.

## The five answers

1. **Can a Claude session open PRs under its own identity?** Locally run Claude Code: yes,
   with a GitHub App installation token in `GH_TOKEN` and the git author set to the App's
   bot (Nano chose a GitHub App on 2026-11-02, `mem_fb86e90e49d845fea7ed3e911cc3430a`). Cloud sessions: not through the built-in
   connection, which is Nano's account. A cloud environment could hold the App's private
   key as an environment secret and push with a minted token, but the GitHub MCP tools
   would still act as `baileyrd`. So the App fixes authorship of pushes and PRs opened from
   that token, not every action a cloud session can take.
2. **The identity.** One GitHub App per agent family (`claude`, later `hermes` if needed);
   Dottius keeps `dottiusmaximus`. Repository permissions: contents write, pull requests
   write, checks read, metadata read. Not admin, not a code owner, no administration or
   workflow write. Installed on the Rusty-Mill repos only. The private key lives outside
   every repo; agents get a 1-hour installation token, never the key, except where an
   environment secret is the only way in (cloud), which scopes it to that environment.
3. **CODEOWNERS.** No change. Card #24 asked for owner review on engine, schema and log
   paths, but M-merge (2026-11-10) says gates should not be human approvals. Owner review
   would put Nano back in the merge path. The ask-first list (migrations, deletions,
   dependency and toolchain bumps, public API changes) stays a rule agents follow and the
   reviewer checks, pending M5.
4. **Ruleset change** (all under the main ruleset's pull request rule):
   - `required_approving_review_count`: 0 to **1**. With one identity per family, the
     author cannot approve its own PR, so the approval comes from the other family or
     from Nano. This is what enforces cross-family review on GitHub.
   - `dismiss_stale_reviews_on_push`: **true**; `require_last_push_approval`: **true**, so
     the approval covers the head that merges.
   - `require_code_owner_review`: stays **false** (answer 3).
   - Bypass actors: add **Nano (repository admin role) as bypass "for pull requests
     only"**, as the emergency path. No agent identity gets bypass.
   - Tooling-only PRs need the same one approval; the merge queue then runs as today.
5. **Throwaway-PR test plan** (also the GitHub half of the Phase 1 gate demo):
   - A. The App opens a docs-only PR. With CI green and no review it cannot be queued.
   - B. Dottius approves it. It can now be queued and merges through the queue.
   - C. The App approves a PR it authored: GitHub refuses the self-review.
   - D. A push after approval dismisses it; the PR cannot be queued until re-approved.
   - E. Check that an App's approval counts toward the required count (GitHub's docs do not
     say so outright) by having the App approve a Dottius PR.
   - Close the throwaway PRs unmerged except B.

## The gap Nano needs to decide

While cloud sessions act as `baileyrd`, a cloud Claude review counts as Nano's approval,
which defeats answer 4 for every PR Dottius authors, and the session holds Nano's admin
rights. Options:

- **a. Cloud Claude never approves on GitHub; it comments only.** A rule in AGENTS.md,
  checked by the reviewer. Cheapest; enforced by discipline, not by GitHub. Cost: until
  the App exists, Dottius's PRs need Nano's approval.
- **b. Claude's builder work moves to the App identity.** Cloud environments mint App
  tokens from an environment secret for git and PRs; the MCP connection is kept read-only
  in use. Enforced for authorship, still discipline for the MCP tools.
- **c. Nano uses a separate personal admin account,** and `baileyrd` drops to write
  access. Enforced by GitHub, but a large change to his own setup.

Recommendation: **a** now, **b** when the App exists. c only if a or b fails in the gate
demo.
