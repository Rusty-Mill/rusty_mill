# Proving run: first live task

The first task driven end to end by live models under `bbp mod`: four Claude Code harnesses as Planner, Coder, Tester and Reviewer, one human at the gates, one sandboxed runner. Its purpose is a record, not a pass: where the rules bit, what the budgets hit, where a role stalled. That record answers the spec's open questions Q1 to Q3 (`docs/spec/bbp-v0.4-implementation-spec.md`, "Open questions for after the proving run"). Never raise a limit to make the task pass.

Files in `docs/proving/`: `brief.md` (the task), `profiles.json` (the frozen test profile), `agents.json` (one launcher per role). Paths in the last two assume `/tmp/bbp` and a user `nano`; edit before use.

## 1. Build

```sh
cargo build --release -p rusty_bbp_host
export PATH="$PWD/target/release:$PATH"   # `bbp` on PATH: agents.json launches it by name through {mcp_config}
```

## 2. Target repository

A throwaway crate with no dependencies, so `cargo test --offline` works with the network blocked.

```sh
mkdir -p /tmp/bbp && cd /tmp/bbp
cargo new --lib target && cd target
mkdir tests && cat > tests/slug.rs <<'T'
use target::slug;
#[test] fn lowercases_and_joins() { assert_eq!(slug("Hello, World!"), "hello-world"); }
#[test] fn collapses_runs()      { assert_eq!(slug("a  --  b"), "a-b"); }
#[test] fn trims_edges()         { assert_eq!(slug("--x--"), "x"); }
#[test] fn empty()               { assert_eq!(slug(""), ""); }
T
cargo build -q && cargo test -q 2>&1 | grep -q 'unresolved import' && git add -A && git commit -qm "slug: failing tests"
cargo fetch   # warms ~/.cargo so the sandboxed `cargo test --offline` needs nothing from the network
```

## 3. Open the task

```sh
export BBP_DIR=/tmp/bbp/store BBP_TASK=slug-1
P=crates/apps/rusty_bbp_host/docs/proving
bbp open   --repo /tmp/bbp/target --brief $P/brief.md --human nano --profiles $P/profiles.json
bbp assign --role planner  --principal planner  --vendor anthropic
bbp assign --role coder    --principal coder    --vendor anthropic
bbp assign --role tester   --principal tester   --vendor anthropic
bbp assign --role reviewer --principal reviewer --vendor anthropic
```

`profiles.json` must name the toolchain the workload needs: `read_roots` with `~/.cargo` and `~/.rustup`, `PATH` with `~/.cargo/bin`. The sandbox allows nothing else, and the set is frozen at open: a change afterwards means a new task.

## 4. Run

Shell A, the moderator (returns at `closed` or `cancelled`):

```sh
bbp mod --repo-path /tmp/bbp/target --work /tmp/bbp/work --agents $P/agents.json --max-wall-secs 7200
```

Shell B, the human:

```sh
watch -n 5 'bbp card | python3 -m json.tool | head -40'
bbp human approve-plan  ART                 # at plan_gate: ART is the spec's artifact id from the card
bbp human approve-merge ART RUN             # at merge_gate: the candidate and its selected run
git -C /tmp/bbp/work/run-RUN diff --cached > /tmp/bbp/cand.diff   # the runner's checkout holds the applied candidate
git -C /tmp/bbp/target apply --index /tmp/bbp/cand.diff && git -C /tmp/bbp/target commit -qm "slug (BBP slug-1)"
bbp human receipt ART "$(git -C /tmp/bbp/target rev-parse HEAD)"   # closes the task
```

States in `bbp human` are lower snake case (`planning`, `plan_gate`, `build`, `test`, `review`, `merge_gate`, `approved`, `escalated`); the card prints them capitalised.

Destructive verbs (`reject`, `rerun`, `resume`, `cancel`) take `--rev N`, the card revision you are looking at. A stale revision is refused; re-read the card and decide again.

Logs: `$BBP_DIR/agents/<sha256(task)>/turn-N.log` per harness, `$BBP_DIR/mcp/<sha256(task)>/turn-N.json` the config each saw, `/tmp/bbp/work/run-N` the runner's checkouts.

## 5. Record

Write `docs/proving/<date>-slug-1.md` with, in this order:

1. Outcome: `closed`, `cancelled` or `escalated`, wall time, number of turns, iterations, candidates, runs.
2. The card's final `budget` block next to the limits, and `rejections` by code. For every limit hit: which role, on which turn, doing what. This is Q1's input.
3. Each role's turns: did it finish its job in one turn, what did it read first, what did it get refused, where did it stall or loop. Quote the log line, not a paraphrase.
4. Human interventions: every `bbp human` command with its reason.
5. The Reviewer's verdict on each candidate, next to the Tester's. Agreement or not is Q3's first data point.
6. Anything the harness did that the protocol did not see (shell use, repository writes outside the diff). This decides whether harness isolation comes next.

Out of scope for this run: the 16-task comparison against the relay loop (spec, "Measurement"). That needs a published task set and rubric, and comes after the first record exists.
