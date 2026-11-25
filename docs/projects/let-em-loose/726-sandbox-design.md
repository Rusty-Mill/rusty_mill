# Project sandbox and tool gateway: umbrella design (M3, cards #726 / #1407)

Let Em Loose, 2026-11-25. Status: **approved by Nano, 2026-11-25** (D180; O1 a = D181, O2 a = D182; O3 and O4 defaults = D183, D184). Satisfies D25 for #1375–#1382 and #1412, not #723; no §11 ask-first item is approved.
Security-sensitive (tokens, identity, isolation, egress): independent review requested.

One contract for the M3 step cards, so each builds against the same token, policy and layout:
#1375 session binding, #1376 identity upstream, #1377 upstream modes, #1378 lens policy,
#1379 fs/shell server, #1380 layered workspace, #1381 skill layers, #1382 orch-minted tokens,
#1412 the jail layers left over from D107. #723 (lens composition) produces the policy later.

## 0. What it builds on

- **D101** [stated]: tools go through the gateway; the agent process is jailed; no secrets inside, only a
  short-lived token scoped to lens and claim; audit from a witness outside the jail.
- **D102** [stated]: orch decides (composes the lens, picks the model, mints the token), the gateway
  enforces. Separate processes; neither imports the other. Shared contract: the policy schema plus the
  token's claims, kept in Factory.
- **D104 item 7 / D107**: `lel-jail` v0 runs on rusty_sandbox, seccomp and a cgroup; the #726 layers
  (namespaces, overlay, gateway-only egress, witness) come later behind the same `jail.json`. D107's
  known gap: `InternetOnly` egress.
- **D160–D162** (#1571 launch profiles): under `jail` the agent gets only D101's token in the `jail.json`
  `token` slot; the profile's service credentials are ignored. Profiles live in Factory.
- **D165/D166**: v0 builders use Nano's subscription through a read-only `~/.claude` root. That exception
  **expires when the gateway lands**; the gateway then holds the login.
- **D133**: packs load lazily, a role is defined by its lens, a domain pack will replace the project expert
  once one exists and Nano confirms the swap (Q47); the expert stays until then. **#723** has its own design step;
  this design does not need it (§5).
- **D104 item 8**: lens sessions in blocks are decided "when #726 reaches lens sessions". This design
  does not reach them: lenses stay one-shot and orch-side. Everything here is for builder sessions; the
  `critic` role in §5 is reserved for when item 8 is decided.
- **No step card has an approved design yet** (#1375–#1382 and #1412 have no design comments), so there
  is nothing to cite.
- **mem_931824b0** (Nano, 11-18): agents running uncontained is fine this phase. M3 is on its own track
  and blocks nothing in M1/M2.
- Code on main (`30d6d6d3`): `rusty_agent_gateway` already validates JWTs against a file JWKS with a
  pinned algorithm list and a list audience check (`agentgateway-auth`), filters tools by regex and CEL
  over `jwt.*` on list and call (`gate.rs`, `rules.rs`), and passes the verified claims to handlers as
  `Caller.principal`. `rusty_mcp_server::session_of` (`http.rs:545`) still checks only the session id
  (#727 gap 8 is open). Upstream connections are shared per target (`HTTP_CONNECTIONS = 8`,
  `target.rs:40`; one stdio child).

## 1. Shape

```
 orch (decides)                       gateway (enforces)                 upstreams
 ─────────────                        ──────────────────                 ─────────
 resolve lens policy ── digest ──┐    verify token (iss, aud, exp, jti)   Mill Board   pooled
 mint token(lens, claim, policy) │    pick policy by lel.lens + digest    remind-me    pooled (read v0)
 write jail.json (token slot)    │    filter list / refuse call           GitHub       per_credential
 Launch via mux → lel-jail ──────┘    stamp identity, strip agent's       sandbox fs   per_session
        │                             upstream session per mode           browser      per_session
        ▼                             log every call (hash chain)         model API    LLM pillar (D166)
 jailed agent ── only egress: gateway ──┘
        ▲
 witness (outside the jail): upper-layer diff, exec log by cgroup, gateway call log → digests on the claim
```

- **One gateway per host** (BaileyAi), serving every project; the project is a token claim, not a process.
- **One workspace per project** (§7), shared read-only by every agent session in it.
- **Models stay outside** the sandbox. A CLI builder (Claude, Codex) runs inside `lel-jail` and keeps its
  built-in tools there (D101); a builder driven by an API-only model (local Qwen) gets the same workspace
  through the fs/shell server (§6). Both reach everything else only through the gateway.
- **agent_cloud microVM** around the whole project sandbox: optional, required for untrusted agents
  (D101). Nothing here depends on it.

## 2. Token (#1375, #1382)

**Format.** A JWT, ES256, verified by the existing `jwtAuth` path against a file JWKS (`FileJwks` accepts
RS256 and ES256, `file_jwks.rs:94`). orch signs with the workspace's vendored `jsonwebtoken` (`ecdsa`),
so no new dependency and no gateway change (§11).

| Claim | Value |
|---|---|
| `iss` | `lel-orch@<host>` |
| `aud` | `lel-gateway` (the gateway refuses any other audience; the confused-deputy test exists) |
| `sub` | the agent session: `<project>/<claim>/<lens>/<n>` |
| `jti` | random 128-bit id; the revocation key |
| `iat`, `exp` | see lifetime |
| `lel.project`, `lel.task`, `lel.claim` | where the session works |
| `lel.lens`, `lel.role` | lens id; `builder` or `critic` |
| `lel.policy` | `sha256` of the policy file's bytes (§5): binds the token to the lens **config**, not the instance |
| `lel.grants` | optional list of widening grants Nano approved through the inbox (§5.3) |

**Lifetime.** `exp` = the session's wall clock (the #600 builder budget, 45 min today) plus 5 minutes, and
never more than 4 hours. orch also revokes early: when the claim settles or the session is dismissed it
appends the `jti` to `revoked.jsonl` (a file the gateway reloads on change), and the gateway drops that
token's upstream sessions (§4). A long session gets a fresh token with the same `sub`.

**Keys.** The signing key is a file on the orch host, outside every `jail.json` read root, so Landlock
keeps jailed agents from it. File permissions alone protect nothing while agents run as Nano's user
(D161); the mount namespace (#1412) or `agent_user` closes that. The gateway holds only the JWKS (public). Rotation: a new `kid` goes into the JWKS next to the old
one; the old one leaves after the longest `exp` has passed.

**Delivery.** The existing `jail.json` `token` {`file`, `var`} slot (D107). No new field.

**Session binding (#1375).** `rusty_mcp_server` records `(iss, sub)` of the creating request's principal
on the session. A request naming that session with a different principal, or with none, gets the same
`404 unknown or ended session` as an unknown id, so a guesser learns nothing. A session created without
a principal stays usable only without one, which keeps today's unauthenticated servers unchanged.
Binding to `sub`, not `jti`, lets a refreshed token keep its session.

## 3. Identity upstream (#1376)

**Header template, not token exchange.** Exchange needs a token service nobody runs; every upstream we
expose today is ours and sits on loopback or the private network. So the gateway authenticates to each
upstream with **its own** credential and states who it acts for:

- **Mill Board.** The gateway holds one bearer credential `lel-gateway` with a narrow role. It strips any
  `actor`, `role`, `agent_id` and `agent_key` argument from a board tool call and sets
  `actor = lel-gateway/<label>`, with the label built from the token (`<lens>-<claim short id>`,
  ≤40 chars, Mill Board's existing `/label` rule). No Mill Board change.
- **remind-me.** Read tools only in v0. Writes wait until rusty-remind-me has scoped tokens or a trusted
  `written_by` header (the same gap #1571 §3 records); it is a rusty-remind-me card, not changed from here.
- **Generic.** A per-target `identity` block: `strip` (argument names, per tool glob), `set` (argument
  name → claim template) and `headers` (header → claim template). It reuses the existing transform module.
  Identity values come only from the verified token, never from the call.

## 4. Upstream session modes (#1377)

Per target, `mode: pooled | per_credential | per_session`, and `connections: N` (default 8, today's const).

- `pooled`: shared, identity per call (Mill Board, remind-me, web fetch).
- `per_credential`: one pool per upstream credential (GitHub, per D102).
- `per_session`: one connection, or one stdio child, per `(downstream session, sub)`.
- **A target with no `mode` is `per_session`** (D102 [stated], "unknown servers default to per-session").
  This changes behaviour for existing agentgateway configs (more connections); `Config::lint` reports
  every target that took the default, so it stays visible.
- **Teardown.** A new `ToolSource` hook, `session_ended(session)`, fires on `DELETE`, idle timeout, token
  expiry or a revoked `jti`. Per-session upstreams close; stdio children are killed and reaped.

## 5. Lens policy (#1378; #723 produces it later)

### 5.1 Schema

One TOML file per lens in Factory, `factory/lens-policies/<lens>.toml`, next to the #1571 profiles (D162).
Hand-written for now; #723's composer generates the same file later, so neither side waits for the other.

```toml
schema = 1
lens = "builder-rust"
role = "builder"                    # builder; critic reserved (D104 item 8)

[tools]                             # federated tool names, glob
allow = ["sandbox_*", "board_card_get", "board_card_comment", "github_create_pull_request"]
deny  = ["github_merge_*"]

[workspace]
view = "worktree"                   # critic: "read_only"
layers = ["rust"]                   # tool layers mounted read-only (§8)

[skills]
packs = ["rust", "terminal"]        # knowledge served as resources (§8)

[grants]                            # widening; anything here needs Nano through the inbox (§5.3)
network = []                        # hosts beyond the gateway
push = false
secrets = []
```

### 5.2 Selection and compile

The gateway loads every file in the policy directory and compiles each into the existing `allowTools` /
`denyTools` patterns plus one CEL rule `jwt["lel.policy"] == "<digest>"`. A token selects the file by
`lel.lens`; the digest must match. **An unknown lens or a digest mismatch denies everything** (empty
list, every call refused). It is a gateway-local config extension (`lelPolicy: {dir}`), reported by lint
like the other non-upstream fields. orch computes the digest from the same bytes, so the two share only
the file and the claims (D102).

### 5.3 Tool authority

The card body names the project expert as tool authority. This design proposes keeping a model out of
the grant path (O1, Nano's call). **Grants inside a role's scope are what the
lens policy already lists, and a policy changes only by a reviewed PR to Factory.** Anything that widens
what can go wrong (a `[grants]` entry: egress host, push, a secret) is an escalation on the board before
orch mints; Nano's approve in the inbox adds it to that one token's `lel.grants`, for that session only.
See open question O1.

## 6. Sandbox fs/shell server (#1379)

For builders that bring no local shell (API-only models). It is a `per_session` stdio target
of the gateway, and **it runs inside the same `lel-jail` and `jail.json` the session would get**, so there
is one containment path. The gateway builds its launch from the token's claims through a fixed layout
(§7), never from call arguments.

- **Tools:** `read`, `list`, `stat`, `write` (builders only), `exec` {`argv`, `cwd`, `timeout_s` ≤ 600,
  `stdin`}, output capped at 1 MiB per stream, exit status returned.
- **Paths:** relative to the session root; absolute paths and `..` are refused before any I/O. Landlock
  and the mount namespace are the real boundary (symlinks, `/proc`).
- **Shape:** a pure core (path rules, caps, result shaping) with an fs/process adapter; a new crate under
  `crates/apps/rusty_orch/crates/` (name settled on the card).
- **Audit:** the gateway's call log already records every call; the server adds nothing of its own.

## 7. Layered workspace (#1380)

On the host, per project:

```
/srv/lel/<project>/base/objects.git        bare repo, read-only to agents; orch fetches outside any jail
/srv/lel/<project>/base/toolchain/         CARGO_HOME, rustup, read-only
/srv/lel/<project>/base/sccache/           shared compile cache, read-only to agents (§7, O2)
/srv/lel/<project>/layers/<name>@<digest>/ tool layers, read-only (§8)
/srv/lel/<project>/views/<commit>/         read-only checkout per candidate commit, shared by critics
/srv/lel/<project>/sessions/<claim>/<n>/   upper/, work/ for the session's overlay; removed on dismiss
```

- **Builders** get a writable worktree: a clone with `objects/info/alternates` pointing at the base (no
  object copies), checked out into the session's upper layer. Pushes go through the gateway.
- **Critics** (once D104 item 8 allows critic sessions) get `views/<commit>` read-only and no upper layer.
- **sccache:** builders read the shared cache and write to a per-session cache in their upper layer.
  The shared cache is refreshed only by orch's own build of `main` outside any jail (O2).
- **Budget and cleanup:** `limits.file_bytes` caps an upper layer; orch deletes `sessions/<claim>/<n>`
  after the witness has read the diff. Reflink forks (agent_cloud ADR-018) are a later optimisation.

## 8. Skill layers (#1381)

Two kinds, both named by the policy (§5.1) and loaded lazily (D133):

- **Knowledge packs** (Factory `knowledge/<pack>/`): served by a gateway-local resource source as
  `skill://<pack>/<file>`, filtered by `skills.packs`, each listing carrying the pack's digest.
- **Tool layers** (`layers/<name>@<digest>`): directories orch adds as extra read-only lower layers of the
  session's overlay.

Ai Env Manager will catalog both (D133); until it does, the directory is the catalog.

## 9. Jail layers (#1412, D107's gap)

Added to `lel-jail` behind `jail.json`, in this order of setup:

1. **Namespaces:** user, mount, network, pid, ipc, uts. The mux socket path no longer exists inside,
   which completes mux-hosted-agents-design §4's test.
2. **Mounts:** an overlay of base, layers and the session upper at the worktree path; `jail.json` gains
   `mounts` (lower list, upper, work, target), validated like the existing roots.
3. **Seccomp and Landlock** stay as in v0 (D107), now over the mounted paths.
4. **Egress: gateway only.** The network namespace has loopback only. `lel-jail` keeps a small forwarder
   in the host namespace that accepts on the jail's `127.0.0.1:<port>` and connects only to the gateway's
   one address. No slirp or pasta dependency. Once #1412 lands, `gateway_only` is the only egress: no
   `jail.json` value can bring back `InternetOnly`, keeping D107's rule that the file can't weaken it.
5. **Model traffic** goes through the gateway's LLM pillar (`agentgateway-llm`), with the CLI's base URL
   pointed at it. The gateway holds the provider login, so D166's read-only `~/.claude` root goes away
   here, as D166 says it must. **Risk:** whether the Claude CLI's subscription OAuth works through a
   base-URL proxy is unverified; #1412 proves it with a spike before relying on it.
6. **Witness:** after the session, orch hashes the upper-layer diff; the gateway's call log is
   hash-chained (#722); the exec log by cgroup needs audit/eBPF on the host (§11). Denials and
   out-of-scope writes post at once. How they post is §12 O3.

## 10. Build order

#1375 → #1376 → #1377 → #1378 → #1379; #1380 after #1318; #1381 after #1378; #1412 after #1380;
#1382 last. Each is one small PR (stacked where dependent) with its own tests, as the card prompts list.
**Approving this design satisfies D25 for #1375–#1382 and #1412** (not #723): their cards then go
straight to code. A step card that departs from this design posts a delta for approval first. Approval
approves no §11 ask-first item; each needs its own yes.

## 11. Ask first (needs Nano's yes before that slice, not now)

- **Public API:** `rusty_mcp_server` session binding (behaviour) and the new `ToolSource::session_ended`
  hook; new gateway config fields `mode`, `connections`, `identity`, `lelPolicy`; new `jail.json` fields
  `mounts` and `net`.
- **Default change:** targets without `mode` become `per_session` (D102), visible through lint.
- **Dependencies:** none new. orch uses the vendored `jsonwebtoken` already in the workspace.
- **BBP schema:** none if O3 takes the recommendation; any new evidence kind or field is ask-first.
- **Host writes on BaileyAi:** `/srv/lel/` layout, the gateway service, the signing key file,
  unprivileged user namespaces if disabled, and audit/eBPF for the exec log.
- **Deployment** of the gateway.
- **Credentials:** the gateway's `lel-gateway` Mill Board credential, and how the gateway stores and
  forwards Nano's provider login (ends D166's exception; see the §9.5 risk).

## 12. Open questions

- **O1. Tool authority** (§5.3), for Nano. The card body names the project expert. Options: (a) reviewed
  Factory policy PRs for in-scope grants, Nano's inbox approval per token for widening, no model in the
  grant path; (b) the expert (later the domain pack) approves in-scope grants, the inbox handles
  widening; (c) only Nano, for every grant. Recommendation: (a), deterministic and auditable.
- **O2. sccache writes** (§7), for Nano; it rewords D101. D101 lists "content-keyed writes for builders",
  but sccache keys are input hashes, so a jailed builder could write a poisoned object under a valid
  key that every later build reads. Options: (a) per-session writes only, orch's trusted build of `main`
  refills the shared cache; (b) shared writes verified by a reproducible rebuild; (c) no sccache in the
  jail. Recommendation: (a).
- **O3. Witness digests as evidence** (§9.6), default proposed. Every BBP evidence kind (`Executed`,
  `Cited`, `Opinion`) scores, and a clean digest is not support. D101 settles that a witness/ACP
  mismatch is evidence against the claim. Proposed: denials, out-of-scope writes and digests post on the
  claim as non-scoring messages; only a mismatch posts as `cited` evidence in the `counter` direction,
  citing the gateway call-log artifact (addendum C-R4). No schema change.
- **O4. remind-me writes** (§3), default proposed. This departs from #1407's prompt ("`written_by` set
  from the token"): D102 stays the target, and until rusty-remind-me can take an identity, writes from
  jailed agents fail closed.

**Programme note.** M3 runs ahead of M2 (#600), and none of its code needs the M2 run
(mem_931824b0). #723 is not part of this approval; it has its own design step.
