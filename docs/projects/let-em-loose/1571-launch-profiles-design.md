# Agent launch profiles: design (card #1571)

Let Em Loose, 2026-11-18. Status: **approved** by Nano 2026-11-18 (DECISIONS.md D160; v0 isolation D161; file location D162). Security-sensitive (credentials, isolation).

## 0. What it builds on

- **Card #1571** (Nano, voice, 2026-11-18) and its comment tying it to D101/#726.
- **Direction** (remind-me `mem_b563d134`, Nano 2026-11-18): Let Em Loose will fold into the mux server.
  Profiles are data, orch is a client of the mux socket, so the merge is a move, not a rewrite.
  Principle: **the mux grants at launch, each service revokes on its own, the orchestrator holds both keys.**
- **No sandbox dependency** (remind-me `mem_931824b0`, Nano 2026-11-18): agents running uncontained is
  acceptable this phase. Profiles do not wait for the jail (D101) or agent_cloud (ADR-025).
- **Layer 8 of the agent stack** (remind-me `mem_ddd0b3c0`, Nano 2026-11-17): tools are what the lens asks
  for plus what the project adds, capped by the agent profile; a project can't widen an agent's trust;
  set at launch; a mismatch fails at launch.
- **Mux side, built or approved:** C75 exact env and passed fds, C76 signal/release, C77 input gate,
  C78 hold, C46 ownership, the `orch-runner` account server (AGENT-HOSTING.md C79 item 4, D104 item 6).
  orch reaches the mux only as a `rusty-mux-ctl own` child (AGENT-HOSTING Gates).
- **Launch metadata** D336 (`agent`, `role`, `card` on `Launch`; approved in principle; the wire slice is
  open as PR #1800, owner gate) and **re-run** D338 (agent blocks re-run through orch and the profile, never by replaying
  argv): rusty_term handoff, Nano 2026-11-18 (#1798). The **dismiss message** (card id,
  evidence links, label) is a design line in `docs/projects/mux/AGENT-ARCHIVE.md`; unbuilt.
- **Built on main** (rusty-term-expert, 2026-11-18): C46 `OWNERSHIP` bit 35, C77 `INPUT_GATE` 36, C75
  `AGENT_LAUNCH` 42, C78 `HOLD` 43, C76 `SIGNAL_RELEASE` 44, all through `rusty-mux-ctl own`.
- **Existing launch path** D104 / mux-hosted-agents-design.md §2: orch prepares worktree, cgroup and token,
  writes `jail.json` (ADR-0022, D107) and sends an exact-env `Launch`. That is this design's `jail` level.
- **GitHub Apps per agent** created by Nano 2026-11-15 (remind-me `mem_f85de011`, Millwright O2/G1).
- **Mill Board** `agent_enrol` (PR #1664, merged 2026-11-13, not deployed): child bearer tokens with
  `agent_enrol_revoke`.

## 1. Shape: two files compose, not one file per pair

A file per (agent, project) pair grows as N×M and lets a project quietly widen what an agent holds.
Layer 8 already answers this for tools, so profiles follow it. The card's "(agent, project) →
environment" is the mapping; it doesn't need a file per pair.

- **Agent file** (trust; once per agent): program, args, the *least* isolation it may run under, and the
  credential kinds and tools it may ever hold (a cap).
- **Project file** (needs; once per project): repo and base ref, toolchain, extra environment, the
  isolation the project demands, the credentials it needs with their scopes, and the tools it adds.
- **`resolve(agent, project, lens, card) -> Result<LaunchPlan, Refusal>`** is pure. Isolation is the
  stricter of the two files. Tools are the lens's ask plus the project's additions; credentials are the
  project's ask. Anything outside the agent's cap is a `Refusal` at launch, never a silent narrowing.

```jsonc
// factory/agent-profiles/claude-builder.json
{ "agent": "claude-builder", "program": "claude-code-acp", "args": [],
  "isolation": "same_user",                      // floor: same_user < agent_user < jail
  "credentials": ["mill_board", "github", "remind_me"],  // cap
  "tools": "…"                                   // layer 8 cap, owned by agent-stack-spec
}
// factory/project-profiles/rusty_mill.json
{ "project": "rusty_mill", "repo": "/srv/repos/rusty_mill", "base": "main",
  "isolation": "same_user",
  "toolchain": [ { "bin": "cargo" }, { "bin": "rustup" }, { "bin": "python3", "min": "3.12" } ],
  "env": { "CARGO_TARGET_DIR": "/srv/agent-target/rusty_mill" },
  "tools": [],
  "credentials": {
    "mill_board": { "projects": ["let_em_loose"] },
    "github":     { "repo": "Rusty-Mill/rusty_mill", "permissions": { "contents": "write", "pull_requests": "write" } },
    "remind_me":  { "project": "let-em-loose" } } }
```

Rust's pin stays the repo's `rust-toolchain.toml`; the profile only says `rustup` and `cargo` must exist.

## 2. LaunchPlan: what orch sends

One resolved plan, sent as one C75 `Launch` through a **fresh `rusty-mux-ctl own` child per agent
launch** (`own` passes only descriptors it inherited at startup, each once).

| Field | From |
|---|---|
| target | a window UUID: `own` launches only into a window, so orch's adapter first finds or makes the project's session and window with one-shot ctl calls (`list`, `create-session --name`, `create-window`). One session per project is orch's convention; the mux doesn't enforce unique names. Server: `same_user` Nano's own; `agent_user`/`jail` the `orch-runner` account server |
| program, argv | agent file; for `jail`, `lel-jail <jail.json>` exactly as D104 §2 |
| cwd | a fresh worktree of `repo` at `base`, one per card |
| env (exact) | `PATH`, `HOME`, `LANG`, `TERM`, the project's `env`, and the minted credentials (§3). Nothing from orch's own environment, no `RUSTY_MUX_*` (ADR-0022/D107) |
| fds | 3 and 4 under `jail` (D104); none otherwise |
| `hold`, `input` | `true`, `owner-only` (D104) |
| `agent`, `role`, `card` | agent name, lens, card (D336, once its wire slice, PR #1800, merges) |

**Preflight** runs before the `Launch`, as the target user, with the plan's exact `PATH`: each toolchain
`bin` must resolve and, where `min` is set, its `--version` must meet it. A miss refuses the launch with a
message naming the tool, posted as a card comment. Nothing starts half-equipped.

## 3. Credentials: mux grants, services revoke

**Credentials go in the exact `env` of the `Launch`, never in `argv`.** The mux persists `argv` (C45
`Started`, unencrypted state dir D74) but never `env` (AGENT-HOSTING §C75). A descriptor-to-env shim
would end with the same variables in the same process, so it adds a binary and buys nothing. If an agent
echoes a token, the archive's transcript scan marks the record sensitive (D339).

**Not in the jail.** Under `jail` the agent gets only D101's one token scoped to its lens and claim, in the
existing `jail.json` `token` slot; board, push and PR go through the gateway. Putting service credentials
inside the jail would reverse D101, so a `jail` profile ignores the project's `credentials`.

| Credential | Mint (per launch, per card) | Scope | Revoke | Exists? |
|---|---|---|---|---|
| Mill Board | `agent_enrol` child `orch.<card>` | the project's board projects | `agent_enrol_revoke` | merged #1664, not deployed; an admin must create the enrol role |
| GitHub | installation token from the agent's own GitHub App (`mem_f85de011`) | one repo, listed permissions | `DELETE /installation/token`; 1 h TTL backstop | Apps exist; their keys must be on the host orch runs on |
| remind-me | per-agent token | one project | its revoke | **no record of scoped tokens** (one read-write key today): a rusty-remind-me card |
| Model provider | — | — | — | LEL Q15 answered (D165: Nano's subscription); delivery into the jail still open |

**Lifecycle.**
1. *Launch:* mint every credential, record `(block, card, credential ids)` in orch-store, then `Launch`.
   A failed mint revokes what was minted and refuses.
2. *Run:* the mux can't take a credential back. Cutting an agent off mid-run is each service's revoke;
   stopping it is C76 `SignalBlock` (and the cgroup under `jail`).
3. *End or dismiss:* orch revokes every recorded credential, then dismisses (card id, evidence, label;
   #1570, AGENT-ARCHIVE.md, unbuilt). A failed revoke is retried and leaves the record open.
4. *orch restart:* sweep orch-store for open records whose block is gone and revoke them. TTLs are the
   backstop for anything orch never sees again.
5. *Re-run (D338):* resolve the profile again with fresh credentials. Never replay the stored argv.

**Lifecycle hooks: deferred.** The card lists them; dismiss behaviour is the same for every agent (#1570),
so v0 has no hooks field. One is added when a second dismiss behaviour exists.

## 4. Isolation levels

**Scoped tokens are not a boundary yet.** Whoever runs orch holds the minting keys (enrol credential,
GitHub App keys). An agent sharing orch's uid can read them. That holds for `same_user` (Nano) and for
`agent_user` (orch runs as `orch-runner`, because the account socket refuses other uids). Nano accepted
uncontained agents for this phase (`mem_931824b0`); the jail (D101) closes it.

- **`same_user`** (proposed v0 default): Nano's uid, his own server. Needs no host writes.
- **`agent_user`** (the card's starting level): the `orch-runner` account server. Keeps Nano's shell env,
  ssh keys and personal tokens away from agents, not orch's minting keys. No mux code change (C46,
  C75–C79 built), but the BaileyAi host writes don't exist: the user, its account-server unit, the front
  door and the `user:baileyrd → orch-runner` map entry (C79 item 4, D228). Each needs Nano's go.
- **`jail`**: exactly the D104 path through `orch-jail`, not a second one.
- **microVM** (agent_cloud, ADR-025): out of scope.

## 5. Where it lives (the seam for the mux merge)

- **Files:** data in Factory, beside `apps/*.json` (D80), which orch already loads:
  `factory/agent-profiles/*.json`, `factory/project-profiles/*.json` (not `agents/`, which collides with
  orch-bbp's `agents.json`, D138). Revisit with Q46 (where pack files live).
- **Types and `resolve`:** a pure module in `orch-core`, serde-free (ADR-0002: no dependencies). JSON
  parsing lives in the orch adapter, as `JailSpec` parsing lives in `orch-jail` (D107).
- **Adapters in orch:** preflight, one minter per service, the session/window lookup, the `own` sender.
  orch stays the only executor (D20 as re-closed by D104).
- **Mux:** nothing new beyond C75–C78, C46 and the D336 fields. Folding Let Em Loose into the mux later
  moves the module and adapters; `Launch` stays the seam.
- **orch-bbp's pool** (`Agent { vendor, program, args, sandbox }`) is the `bbp mod` path, left alone.

## 6. Build order

1. This design, and the principle as `## Grants and revocation` in `docs/projects/mux/AGENT-HOSTING.md`.
2. `orch-core` profile types, `resolve` and its refusals (cloud-buildable, tests only).
3. Preflight, session lookup and the `Launch` sender for `same_user` (Linux, cloud-buildable).
4. Mill Board minter (needs `agent_enrol` deployed and the enrol role: an admin step).
5. GitHub minter (needs the per-agent App keys on the host).
6. `agent_user`, after the host writes. remind-me minter once it has scoped tokens.

## 7. Decided

Nano answered on 2026-11-18 (Let Em Loose project chat, `cmsg_01WgyiM31RsSnZTgNuPX2VE4YGppB9nnPb7JnUCwC3s2Yq`).

1. **v0 isolation: `same_user`** (D161, "Concur"). Agents run as Nano's user now; `agent_user` follows when
   the four BaileyAi host writes are approved; the jail closes the minting-key gap. Rejected: `agent_user`
   now; a separate uid holding the minting keys.
2. **Profile files live in Factory** (D162, "Concur"): `factory/agent-profiles/*.json` and
   `factory/project-profiles/*.json`. Rejected: pack-validation plus repo manifests; this monorepo.
3. **Design approved as written** (D160, "Yes"): composition, env delivery, no service credentials in the
   jail, hooks deferred, build order. Ask-first steps inside it (host writes, dependency bumps) still need
   their own approval.

Non-blocking, to the right owners: which per-agent GitHub App mints for which agent (Millwright); scoped
tokens for remind-me (rusty-remind-me card).
