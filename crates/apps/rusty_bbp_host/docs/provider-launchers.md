# Codex author and Claude reviewer

Build with `cargo build --locked -p rusty_bbp_host`. This produces `bbp`,
`codex-bbp-harness` and `claude-bbp-harness`. These launchers require Linux.
Install/authenticate Codex and Claude Code with their subscription logins; the
launchers do not log in, copy credentials, or support an API-key fallback.

The moderator launches them with its existing per-turn environment. In an
Orch pool use `program: "codex-bbp-harness"` or `"claude-bbp-harness"` and
`args: ["--mcp-config", "{mcp_config}"]`. Add `"--model", "MODEL"` to pin a
model or `"--provider-bin", "/absolute/path/to/cli"` to choose the installed
executable. The latter is a trusted executable selection, not a sandbox for
arbitrary wrappers. Other flags and duplicate options are refused. The
[Orch example](../../rusty_orch/docs/examples/bbp.json) is a two-role roster.
Codex needs a non-final role capable of Spec/Diff/Candidate production and test
verdicts; Claude needs an independent scoped final reviewer with no writes or
raw-log access. Role ids are configuration data, not hardcoded names. Set the
author assignment's `builtin_tools` to true and the reviewer's to false.

Both launchers refuse stale turns, incorrect principals/vendors and foreign or
modified MCP configurations before provider startup. Private files live under
`<BBP_DIR>/harness/<task hash>/turn-N/`; a turn workspace is never reused.
Each invocation starts a fresh provider session and uses only the bound BBP
MCP server. On Linux the launcher replaces itself with the provider process,
preserving the moderator's process-group supervision and the provider's exit
status. Provider startup errors remain errors; there is no scripted fallback.

Codex receives a fresh local clone with no push remote and a workspace-write
sandbox. User config and exec rules are ignored; the clone is untrusted for
project config, hooks/plugins/delegation are disabled, and a required inline
TOML MCP server carries the exact task/principal/turn environment. No global
`codex mcp add` or user-config edit occurs. The author restores prior candidate
diffs when revising and submits artifacts through MCP. The authoritative BBP
runner still applies and validates the candidate independently under confinement.

Claude receives an empty working directory, `--tools ""`, `--restricted`,
`--strict-mcp-config`, empty setting sources, disabled hooks/skills, no session
persistence and `dontAsk` permissions with no interactive permission prompts.
Only task_card/read/get_artifact/post BBP calls are preapproved. The BBP engine
also enforces the scoped, read-only roster and prevents raw-log access. The
launcher, not merely `builtin_tools: false` metadata or the prompt, supplies
the tool restrictions. It refuses a same-vendor candidate writer.

Provider CLIs and administrator-managed policy remain trusted. The author CLI
has local shell/file tools in its own clone; the reviewer has none. This does
not claim an OS sandbox around a malicious provider executable. Host state and
credentials must stay outside the runner workload's granted read roots. API-key
and push-token variables and inherited BBP bindings are removed from provider
environments; the MCP config explicitly supplies only its four public binding
variables. Provider auth stays in the CLI's normal login store.

## Confined

Either launcher may be given a `sandbox` in the agents file. The moderator then
runs the launcher, and through it the provider, inside the harness sandbox. The
launcher cannot read the task directory there. Instead, it reads the facts the
moderator wrote for the turn to a file the sandbox can read but not write (named by
`BBP_LAUNCH_FACTS`), and refuses on the same grounds as when it runs unconfined. The
MCP configuration it gives the provider names only the relay (`BBP_MCP_RELAY`),
and its private files live in the sandbox's `TMPDIR`.

- **Author:** the Codex author clones the task repository with `--shared`, so
  its clone reads the repository's object store and writes its own objects. Grant
  `read` on the repository's `.git` and nothing writable there. The clone stays
  private and fully usable (`git add -N`, `git diff`), and the store cannot be
  altered from inside the sandbox.
- **Provider access:** grant `read` on each provider's installation and login
  store, and `write` on whatever state the provider keeps there. Keep `HOME` in
  `env` if the CLI finds its login through it.
- **Workspace:** leave `checkout` off, since the launcher makes its own
  working directory.

`tests/confined_harness.rs` runs the Codex launcher this way with a provider stub
(acceptance mode `launcher-confined`). The stub cannot read `master` or write the
object store, and it plans through the relay.

## Compatibility and tests

The fixed arguments were checked against installed Codex CLI 0.155.1 and Claude
Code 2.1.302 help. Unsupported flags fail startup; restrictions are never
silently dropped for older CLIs. Contract tests use provider stubs and make no
model calls. Run them with:

```sh
cargo test --locked -p rusty_bbp_host --lib launchers
cargo test --locked -p rusty_bbp_host --test provider_launchers
```

Sources for the command contracts:
[Codex CLI](https://learn.chatgpt.com/docs/developer-commands?surface=cli),
[Codex configuration](https://learn.chatgpt.com/docs/config-file/config-reference),
[Claude CLI](https://code.claude.com/docs/en/cli-reference).

## Opt-in real-provider Linux smoke

This test makes subscription model calls; it is ignored in ordinary CI. Choose
models available to the two installed/logged-in providers. No real merge or push
is performed. The fixture creates a tiny disposable repository and uses the
real Orch CLI, persisted state, production supervisor, confined runner and both
provider launchers through plan approval, candidate validation, independent
review, merge approval and a clearly marked test-only receipt. It fails on a
refused sandbox or missing provider progress. Model behavior is nondeterministic;
retain the temporary task's agent/moderator logs when investigating a failure.

```sh
cargo build --locked -p rusty_bbp_host -p rusty_orch
BBP_LIVE_SMOKE=1 \
BBP_CODEX_MODEL='YOUR_CODEX_MODEL' \
BBP_CLAUDE_MODEL='YOUR_CLAUDE_MODEL' \
BBP_TEST_BIN="$PWD/target/debug/bbp" \
ORCH_TEST_BIN="$PWD/target/debug/rusty_orch" \
cargo test --locked -p orch-bbp --lib \
  tests_confined::opt_in_codex_author_claude_reviewer_smoke \
  -- --ignored --exact --nocapture
```

The five deterministic confined acceptance cases and the separate pause/adoption
regression are required CI. The latter checks repeated Orch processes retain one
task and charge while paused, then adopt after an owner reset; it makes no claim
of confined workload execution. A real deadline-expiry regression also verifies
live harness cleanup and persistent pause after three expired turns. Successful
offline contracts do not establish successful live-provider behavior; report
the smoke's actual result separately.

## Pilot recovery and bounded retries

Codex's `approval_policy="never"` refuses interactive escalation; each of the
five enabled BBP agent tools has an explicit `approval_mode="approve"`. No human
command is an MCP tool. Shell execution keeps the workspace-write sandbox.
Inherited `GIT_*` context is removed before both clone preparation and provider
execution; shell exclusions do the same and set only fixed Git configuration
isolation values. The author cannot accidentally use an external worktree/index.

The moderator stops after three no-progress forfeits/timeouts for a role. It
replays the existing BBP event log: reads and accepted-operation replays do not
count as publishing progress; an artifact/message resets that role's streak.
A paused moderator starts no more providers after an Orch or host restart.
Orch reports the diagnostic as a retryable host error and retains its task and
ledger. This is a launch-loop guard, not a model-token or monetary cap. A provider
that publishes actions or stays alive can still consume its ordinary BBP budget.
The cost/run guards merged in PR #763 remain in force when configured; this
launch-loop guard does not replace their accounting or enforcement.

Inspect and recover explicitly after repairing the launcher:

```sh
bbp mod-health --dir STORE --task TASK
bbp show --dir STORE --task TASK
bbp mod-health --dir STORE --task TASK --reset yes --rev DISPLAYED_REV
```

Reset takes the moderator lock, refuses stale revisions, and records only an
owner-acknowledged replay boundary in `mod/<task hash>.retry`. It does not edit
BBP events, refund turns, change a roster/profile, answer a human gate or restart
a provider. Invoke Orch again to adopt the same task. A changed launcher on disk
can be repaired without changing the task's frozen program/argument configuration;
changes to a pool/roster/profile belong to a new task.

## Rust workloads using LLD

Generate this profile **before opening the task**, then name it in the BBP config:

```sh
bbp profiles rust-lld --sysroot "$(rustc --print sysroot)" > profiles.json
```

It compiles `src/lib.rs` as tests with `-C link-arg=-Wl,--threads=1`, then executes
`.bbp-tests`, under the existing 1 GiB address-space cap and confinement. This
profile requires a Linux Rust toolchain using LLD (the modern x86_64 Linux Rust
default); it is not a flag for BFD/other linkers. Read roots name the resolved
Rust sysroot and system libraries, never the user's credential/Cargo home.
No environment variables or credentials are copied into the workload.
The ordinary generic shell profile is unchanged. The frozen profile and digest
are never edited during recovery. For an existing task, the live pilot instead
narrowed CPU affinity and requested a new run of the unchanged candidate/profile.

The required acceptance collector now records a fifth `rust-lld` case. It must
compile and pass seven tests with enforced confinement and the 1 GiB cap;
Landlock refusal or missing completion evidence fails that case.

Live pilot evidence supplied by the owner used Codex 0.163.0 and Claude Code
2.1.302 with external Python launchers: seven tests passed, an environmental
linker failure recovered without changing the profile/candidate tree, a stale
human answer was rejected, and receipt commit
`6d70d6509de82d930f990e33de49348949bc809e` completed the disposable fixture.
That evidence validates the integration pattern; it is not an execution claim
for these packaged Rust launcher binaries. Their live smoke remains opt-in.
