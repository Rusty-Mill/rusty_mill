# Autonomous maintenance, human-approved execution

```sh
rusty_orch maintain --repo /repo                          # default: dry-run JSON
rusty_orch maintain --repo /repo --bbp /config/bbp.json --execute
# Repeat the execution command to recover/poll the SAME task.
# Approve the proposed spec, or answer an escalation, through bbp human.
# After validation and independent review, optionally authorize draft publication:
rusty_orch maintain --repo /repo --execute --publish-pr
```

Use the [existing provider-neutral BBP configuration](../orch-bbp/README.md).
Execution requires Linux, a clean source worktree, Python 3.11+, Git, Cargo,
installed fmt/clippy components, cached dependencies and a current lockfile.
`gh` must be authenticated to read all open PRs; execution fails closed when PR
state cannot be established. Configure the existing BBP profiles' read roots,
limits, environment and PATH to include Git/Python and the Rust toolchain.
Maintenance preserves those profiles and adds its own required validation.
The packaged Codex author may use built-in tools in its isolated clone, which
has no push remote. Final reviewers remain scoped, read-only and without
built-in tools. Candidates still travel through BBP artifacts and confined
validation; author tools do not grant writes to the source checkout.

Dry-run performs inspection and reports one selection plus reasons for every
exclusion. It reads existing maintenance state, but creates no journal, task,
provider process, build or PR. Current PR/task conflict checks run at execution.
No `ORCH_*` variable can opt into maintenance or publication.

Publication freezes the push destination, GitHub repository and default branch
in the private PR bundle. Before creating a publication worktree, it fetches
that branch and requires the inspected base to belong to its history. A repair
validated on an unmerged feature commit is refused: repeat discovery and
validation on the target branch. Recovery uses the same frozen destination.

The first policy permits directory-layer metadata repairs and identical external
dependency hoists. A hoist remains advisory until a human confirms its benefit:

```sh
rusty_orch maintain --repo /repo --confirm-benefit 'duplicate-declaration:ID' \
  'These two members use identical features/source; centralizing removes drift.'
```

Pass the same confirmation with `--execute` to select it. A human must still
approve the proposed spec through BBP. The sandbox scope profile independently
checks the original evidence and the exact hoist, refusing other manifest edits,
source changes, new/deleted files, renamed files, symlinks and version/feature
changes. Structural dependency findings require a separately designed task.

Selection minimizes risk deterministically: explicit policy defects first, then
reverse validation scope, edit count and finding ID. At most two crates may be
edited and eight validated. Impact is recomputed from inspect's reverse graph
and checked against its versioned impact field. Open PR file conflicts and any
nonterminal BBP task for the repository block new work.

State lives under `$(git rev-parse --git-common-dir)/rusty-orch-maintenance`.
Linked worktrees share the lock/journal. Never delete it to retry: an interruption
is recovered by repeating `--execute`; changing the config does not change an
existing task's frozen roster, profiles, base or identity. If an uncertain open
finds a changed source HEAD, restore the recorded base before resuming. A failed
run stays on the same BBP task, whose public card/report and escalation are
retained. Use `bbp human` to resolve or cancel it. Cancellation is recorded and
that finding is not automatically attempted again. A terminal receipt records
success; no maintenance command fabricates one.

After a passing sandbox run and independent review, the `pr/<namespace>` folder
contains `candidate.patch`, `PR.md` and `report.json`. Without `--publish-pr` there
is no commit, push or PR. Publication creates a private worktree under BBP's
configured work directory, checks the staged AND committed tree against the
validated tree, pushes a stable branch and creates a draft PR. A lost reply is
reconciled against that branch/PR's current head before any retry. Local Git
identity and authenticated Git/gh access are prerequisites. Publication errors
retain the same task and worktree for safe retry; modified publication worktrees
or a changed validated candidate require inspection rather than discarding work.
The task remains active at the human merge gate after PR preparation/publication.
Human approval and external merge/receipt or cancellation release the slot.

## Validation

```sh
cargo test --locked -p rusty_orch -p orch-bbp -p orch-core -p orch-dispatch \
  -p orch-cli -p orch-codex -p orch-claude -p orch-ollama -p orch-store
cargo test --locked -p rusty_bbp --test durable --test human_fence
python3 -m unittest discover -s .github/scripts -p test_maintenance_validation.py
cargo build --locked -p rusty_bbp_host --bin bbp
BBP_TEST_BIN="$PWD/target/debug/bbp" cargo test --locked -p rusty_orch --lib \
  maintenance::e2e::sandboxed_maintenance_recovers_and_prepares_reviewed_pr_without_merging \
  -- --ignored --exact --nocapture
```

The end-to-end test uses the production sandboxed supervisor, real BBP MCP
fixture providers, a separately issued human approval and real Cargo validation.
It loses an open checkpoint, kills/restarts the moderator, verifies one durable
task, prepares a reviewed PR bundle and confirms the source and merge receipt
remain untouched. It does not use live provider subscriptions or publish remotely.
Existing adapters' opt-in live tests remain separate; report their actual results
rather than treating ignored tests as successful live verification.

Implementation-session results and environment limits are recorded in
[MAINTENANCE-VALIDATION.md](MAINTENANCE-VALIDATION.md).
