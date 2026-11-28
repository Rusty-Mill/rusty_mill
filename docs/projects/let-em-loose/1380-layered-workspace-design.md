# Layered project workspace: step design (#1380)

Let Em Loose, 2026-11-28. Status: **draft for Nano.** Fills in `726-sandbox-design.md` §7, which D180
approved. One point departs from it and needs Nano before code (§9 B2); two small deltas are listed
for the record (§9). Everything else is detail inside §7. Code also waits for #1318 (the launcher in use), so nothing here is built yet.

## 0. What it builds on

- **§7 / D180**: the `/srv/lel/<project>/` layout; builders get a clone with `objects/info/alternates`;
  critics get `views/<commit>` read-only; orch fetches outside any jail.
- **D101 / D182**: the shared `.git` and shared build cache holes. Agents never write anything shared;
  only orch's trusted build of `main` refreshes the shared cache.
- **D104 item 8**: no critic sessions yet. This design builds the critic view and its jail roots, and tests
  them, but launches no critic.
- **#1412** owns namespaces, overlay mounts and egress, and is ordered *after* this card (§10 of the
  umbrella). So this card ships the layout and isolation with what `lel-jail` has today (Landlock read and
  write roots), and #1412 swaps the session directory for an overlay upper layer without changing the layout.
- **Code on main (`1b7cec8c`)**: the #1318 builder makes a `git worktree add --detach` worktree of
  `--repo` (`builder/live.rs` `prepare`), gives the jail that worktree as its only write root, and stages
  the candidate outside the jail through the worktree's admin dir (`candidate_diff`, `sealed_git`). By
  default the repo is not a read root, so git does not work inside the worktree at all (its `.git` file
  points at an admin dir the agent cannot read). An operator who passes `--read <repo>` to fix that hands
  the agent the whole repo's `.git`, other worktrees' admin dirs included. That is the shared `.git` hole.
- **Host**: BaileyAi (D53/D131; #754 closed as superseded). Its free disk on `/srv` is not recorded
  anywhere I could find; §6 makes the budget a config value and a probe.

## 1. Layout

```
/srv/lel/<project>/
  base/objects.git/            bare mirror of --repo; orch-runner writes, agents read
  base/toolchain/              CARGO_HOME + rustup, read-only to agents
  base/target/  or  sccache/   the shared build cache (§5, B2)
  layers/<name>@<digest>/      tool layers, read-only (#1381 fills it)
  views/<commit>/              read-only checkout, shared by critics
  sessions/<claim>/<n>/
    wt/                        the builder's clone: write root, cwd
    cache/                     the session's own build cache: write root
    seal.git/                  orch's git dir for the candidate diff: outside every root
    run/                       jail.json, status, logs (as today)
  lock                         flock for base refresh and GC
```

`<project>` may not be one of the names already under `/srv/lel/` (`bbp`, `work`, `orch-state`,
`slots`) or end in `.json`; the layout refuses them. One pure module, `orch_bbp::workspace` next to
`builder` (D228), owns this as typed paths: `Workspace::session(claim, n)`, `view(commit)`, and the two jail-root sets below. It does no
I/O; the git and filesystem calls stay in `builder/live.rs`'s adapter.

## 2. Base: who builds and refreshes it

- **orch, outside any jail, under `lock`.** On admission, orch checks that the session's base commit is in
  `objects.git` (`git cat-file -e <commit>^{commit}`); if not, it runs `git fetch <--repo> <commit>`.
  Nothing else writes the base, and no agent process ever holds the lock.
- **First creation** is `git clone --bare --no-local <--repo>`, so nothing hardlinks back into Nano's repo.
- **Never prune.** Builder clones borrow base objects through alternates, so a prune in the base could
  delete an object a live clone needs. The base gets `gc.pruneExpire=never` and
  `gc.auto=0`; orch runs `git repack -a -d --keep-unreachable` only from GC (§6), when no session is live.
- **Toolchain** is installed by Nano's host setup (ask-first host write), not by orch.

## 3. Builder clone and the candidate diff

```
git clone --shared --no-checkout base/objects.git sessions/<claim>/<n>/wt
git -C wt checkout --detach <commit>
git clone --bare --shared --no-checkout base/objects.git sessions/<claim>/<n>/seal.git
```

- `--shared` writes `objects/info/alternates` and copies no objects. The agent owns `wt/.git` completely:
  it can commit, branch, rebase. New objects land in `wt/.git/objects`, inside its write root.
- **The candidate diff ignores the agent's `.git`.** orch stages with `--git-dir seal.git --work-tree wt`,
  the same `sealed_git` path as today, with `seal.git` in place of the worktree admin dir. `seal.git` is
  outside every jail root, so the agent cannot change its config, hooks or index. Existing checks stay:
  no gitlinks in the base, no nested repository, `.lel/` excluded.
- **Witness.** The upper-layer diff of the done-when line is this staged diff, hashed. Under #1412 the
  overlay upper replaces `wt/` and the same diff is taken from the merged view.
- **Pushes** go through the gateway (§7 of the umbrella); the clone has no remote with credentials.

## 4. Jail roots (today's `jail.json`, no new fields)

| | read roots | write roots | cwd |
|---|---|---|---|
| builder | system roots, `base/objects.git`, `base/toolchain`, `base/<cache>`, its policy's `layers/*` | `sessions/<claim>/<n>/wt`, `.../cache` | `wt` |
| critic | system roots, `views/<commit>`, `base/toolchain`, `base/<cache>` | `sessions/<claim>/<n>/scratch` | `scratch` |

- No session's roots name `sessions/` itself, `seal.git`, `run/` or another session, so Landlock hides
  every other agent's work. The critic's cwd is a scratch dir because `jail.json` requires `cwd` inside a
  write root; the view path goes in its prompt. No `JailSpec` change.
- Same-uid caveat: in v0 agents run as the same user as orch (D161 `same_user`), so file modes protect nothing and Landlock is
  the boundary until #1412's mount namespace or `agent_user`.
- **Mount recipe for #1412** (recorded here so the layout fits it, built there): the builder's root
  becomes `overlay lowerdir=layers/…:views/<commit>, upperdir=sessions/<claim>/<n>/upper,
  workdir=…/work` mounted at the same `wt` path inside the jail's mount namespace; the base and cache
  directories are bind-mounted read-only. The view then holds the checkout once per commit and each
  session's upper holds only what it changed.

## 5. Build cache (B2)

The umbrella names sccache. Two facts found while detailing it:

1. sccache does not cache incremental rustc builds (sccache README, Rust section), and dev builds are
   incremental by default, so a jailed builder would need `CARGO_INCREMENTAL=0`. (Our CI trial also saw
   0 of 22 requests cacheable, ADR-0009, but the cause was never found.)
2. Read-shared plus write-per-session needs sccache's multi-level chain (`SCCACHE_MULTILEVEL_CHAIN`,
   docs/MultiLevel.md, sccache 0.15+), whose levels are backend types (`disk`, `redis`, `s3`, …), with a
   read-only level set by `SCCACHE_LOCAL_RW_MODE=READ_ONLY`; docs/Local.md says local disk supports one
   sccache server at a time. So D182 with sccache needs a second backend on the host (redis or webdav)
   plus the sccache binary. To be rechecked against the version we would install.

**Recommended instead:** a reflink-seeded target dir. orch's trusted build of `main` keeps
`base/target/`; each builder session gets `cache/target` = `cp -a --reflink=always base/target`, set as
`CARGO_TARGET_DIR`. A reflink copy takes seconds (per-file metadata, no data copied) and costs disk only
for blocks the session changes, writes
are private (nothing shared is writable, D182's point), incremental builds keep working, and nothing new is
installed. It saves the third-party dependency builds; workspace crates likely rebuild once per session,
because the session's source path differs from the base build's. It needs `/srv` on btrfs or XFS with
reflink (Fedora's default is btrfs; Q68 checks the box). Without reflink, orch falls back to a cold
`cache/target` and logs it.

## 6. Cleanup and disk budget

- **Session end:** after the witness hashes the diff and the candidate (if any) is posted, orch deletes
  `sessions/<claim>/<n>/`. A crashed session's dir is removed by the next GC once its cgroup is empty.
- **Views:** created on first use per commit, removed by GC when no critic session names them and older
  than 24 h (D230).
- **GC** (a `rusty_orch workspace gc` verb, run by hand or at the start of each builder admission; a
  systemd timer would be a host write, ask-first): sessions with no live
  cgroup, unused views, then the base repack (§2) when no session is live.
- **Budget:** a per-project config `free_floor_bytes`. Admission refuses a new session when the
  filesystem's free space is under the floor (`statvfs`), and the refusal is a session error, not a crash.
  Provisional default 40 GiB (D229), to be checked against BaileyAi's real free space (Q68).
- **Drift noted:** umbrella §7 says `limits.file_bytes` caps an upper layer. It is `RLIMIT_FSIZE`, a
  per-file cap. A hard per-session total needs a quota (btrfs qgroup, or the overlay upper on a sized
  filesystem) and belongs with #1412. This card has the free-space floor only.

## 7. Slices (stacked, each with its own tests; no paid calls)

1. **S1 layout + base.** The typed layout, base create/fetch under the lock, `gc.pruneExpire=never`.
   Tests: fetch is idempotent; a second process waits on the lock; paths refuse `..` and absolute parts.
2. **S2 builder clone.** `prepare` switches from `add_detached` to the alternates clone and `seal.git`;
   jail roots per §4. Tests: real `lel-jail` (Linux, skipped where Landlock is absent, like orch-jail's):
   the agent cannot write into `base/`, cannot list or read another session, can `git commit` in its own
   clone; `cargo build` runs with the read-only `CARGO_HOME` (cargo takes a package-cache lock there, so
   this may need a read-only registry plus a session-local home); two concurrent toy sessions on one base produce two separate staged diffs.
3. **S3 critic view.** `views/<commit>` and the critic's jail file. Tests: a write anywhere in the view
   fails; nothing launches a critic (D104 item 8).
4. **S4 build cache** as Nano picks in B2.
5. **S5 cleanup + budget.** `workspace gc`, the admission floor. Tests: a live session is never collected;
   the base is not repacked while a session is live; admission under the floor is refused.

Done-when witness (card): S2's concurrent-session test plus, on BaileyAi, two toy sessions run by the
#1318 toy script against one base, with their two diff digests compared.

## 8. Ask first (not approved by approving this design)

- **Host writes on BaileyAi:** creating `/srv/lel/<project>/`, the toolchain install, and (if B2 a) a
  check that `/srv` supports reflink. A paste-ready prompt goes to project files when it is time.
- **Public surface:** new orch verb `workspace gc`, builder flag `--workspace <dir>` and config field
  `free_floor_bytes` (additive);
  `--repo` keeps working as the fetch source. No `jail.json`, BBP or Mill Board change.
- **Dependencies:** none (git CLI and `cp --reflink` only). B2 b would add sccache plus a cache server.

## 9. Open points for Nano

- **Note: scope split with #1412.** This card isolates with Landlock roots on a plain session directory
  and records the overlay recipe; #1412 builds the overlay. The card prompt asks for the recipe here, but
  the approved umbrella already puts overlay mounts and the `mounts` field in #1412 (§9.2, §10, §11), so
  the expert answered it (D180); no question for Nano.
- **Delta: critic scratch dir (D231).** Umbrella §7 gives critics "no upper layer". They get a small
  writable `scratch/` as cwd because `jail.json` requires one; the view itself stays read-only.
- **B2. Build cache.** (a) reflink-seeded `target/` per session, no sccache; (b) sccache per D182, with
  `CARGO_INCREMENTAL=0`, a redis/webdav shared level and the sccache install; (c) no shared cache in v1,
  cold builds, decide after the toy run. (a) keeps D182's rule (agents never write shared; only orch's
  build of `main` refreshes it) but swaps the mechanism, and D182 rejected "no sccache in the jail" when
  only sccache options were on the table. Recommendation: (a).
