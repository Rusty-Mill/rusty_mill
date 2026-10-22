# Codex interactive worker: disabled

All three jobs contain literal `false` activation gates. There is no variable,
secret, auto-merge setting, push or merge that can enable them. No provider was run
and no credential was installed/read/changed by this correction. Separate explicit
approval and a reviewed activation PR must choose provider budget/usage limits,
runtime dependency versions, publication scope and a verified publishing identity.
`CODEX_WORKER_LOGIN` is intentionally not configured here. Never invent an account
or substitute the maintainer's identity. The resolver verifies that configured
account through GitHub; the publisher additionally compares the token's live
GraphQL `viewer.login` before any write. Using the default ephemeral GITHUB_TOKEN
must be consistent with that actual publishing principal. A custom App identity
would require a separately approved authentication design, not a PAT workaround.

Only newly created human `@codex` issue comments from an owner/member/collaborator
are candidates; live write permission is refreshed. Bots, edited comments, reviews
and automatic PR events cannot invoke this worker. Generation never publishes
instructions for another agent. A repository-wide queued concurrency group also
serializes an issue request with comments on its later PR. Cancellation is off;
GitHub queue capacity/order limits still apply. No cross-agent loop is enabled.

The read-only resolver fetches current remote heads and bounded issue/PR context
(title/body/latest request plus ten recent human comments). PR targets must be open,
same-repository `codex/*` branches authored by the configured verified
worker; a prefix is only a routing hint. Issue requests first find the existing
`codex/issue-N` ref and matching open PR, then resume its exact SHA. Only when no
branch/open PR/closed PR history exists can a new branch start from current default.
An existing orphan branch, closed prior PR, ambiguous PR or conflicting identity
fails closed for an explicit recovery decision; it is never reset from main.

Generation runs on a fresh GitHub-hosted Linux runner with contents read only,
no PAT or write token, and `persist-credentials: false`. Only the provider action
receives OPENAI_API_KEY; `drop-sudo` protects that input and `:workspace` constrains
commands. The Codex home is outside the checkout. The provider action is the last
step. It returns schema-constrained JSON containing a bounded unified patch through
`final-message`; no runner filesystem, cache, executable or git directory crosses
the boundary. Job outputs may be suppressed by GitHub's secret detection/size
limits; empty/redacted outputs fail closed. No provider key is a publication input.

The publisher is a fresh runner with contents/PR write and issues read via only
ephemeral GITHUB_TOKEN. It loads scripts from the trusted default-branch event SHA,
never from generated code. A separate exact-head checkout has no persisted token.
It validates the JSON/size/path boundaries, applies with `git apply --check --index`,
then accepts only ordinary regular-file additions/edits/deletions (at most 30).
Hidden paths, workflow/configuration paths, AGENTS.md, symlinks, executable modes,
binary patches, renames, copies and traversal are rejected. Hooks, fsmonitor and
global/system Git configuration are disabled; no patched code, tests or builds run
under publisher permissions. Exact indexed bytes become GitHub fileChanges.

Target/head/identity are refreshed both before and after patch validation. New
refs use `createRef`, which rejects an existing racing ref. Commits use
`createCommitOnBranch(expectedHeadOid: original SHA)`, an atomic head comparison,
so a race rejects rather than overwriting intervening work. There is no rebase,
reset of remote history or force push. Existing PRs receive the commit on their
branch, including draft implementation PRs. New issue branches get one draft PR;
an existing matching PR is reused.
If branch creation succeeds but commit/PR creation fails, or an API result is
uncertain, inspect remote state before retry: orphan branches intentionally require
manual recovery. Never delete/reset them automatically. GITHUB_TOKEN publication
can suppress follow-on workflow runs; activation must decide how CI is requested
without broadening credentials or enabling a loop.

Bounds: five-minute resolve, twenty-minute generation, five-minute publication;
200000 patch characters, 30 files. These are not a provider spending cap. Provider
runtime smoke tests and any tool-install/version choices remain deferred to the
activation decision. Non-provider validation here covers policy, real Git patch
application, branch resume, fork/identity rejection and exact-head publication.

Reviewed official sources:
- [Codex permission-profile, output-schema and final-message](https://github.com/openai/codex-action/blob/1d82a3dba7c7988ec71ecae1ed5b8e8db96495c5/action.yml)
- [Codex clean-job, privilege and provider-key guidance](https://github.com/openai/codex-action/blob/1d82a3dba7c7988ec71ecae1ed5b8e8db96495c5/docs/security.md)
- [GitHub createCommitOnBranch contract and expectedHeadOid input](https://docs.github.com/en/graphql/reference/commits#createcommitonbranch)

Validate without providers: `node --test .github/scripts/test_codex_worker.cjs`,
`python .github/scripts/changes.py check` and workflow lint. The Python discovery
bridge runs the Node policy tests in the existing plan-tests CI job.
