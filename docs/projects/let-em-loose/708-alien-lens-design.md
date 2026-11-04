# #708 Alien review lens: design v1

Status: approved by Nano 2026-11-04 (D68). Unparked by D67 (2026-11-04). v0 went through a Goldfish review
(project files `let-em-loose-708/708-goldfish-review.md`: 3 blockers, 7 majors); v1 answers every item, mostly by
taking the reviewer's biggest simplification (M4): **v1 is shadow-only and changes nothing in BBP.**
Depends on #594 (lens runner) and #596 (value score, `Policy`; D36).

## 1. What it is

A shadow lens in the #594 runner that hunts defects the ordinary review lenses are unlikely to find,
and counts a finding only when a sandboxed reproducer demonstrates it. Method from "alien-hunt"
(alien-space-of-science issue #2): ordinary findings are the *available* set; *suspects* are built
from the diff's sites crossed with a defect-mechanism list; the most unavailable, coherent suspects
get a directed hunt.

In v1 the lens **posts nothing to the board**. It records results in orch-store, so we learn whether
it finds anything before it can block a claim or touch BBP. No new executor (D20), no agent-to-agent
calls (D13), no new trigger rule (#596 decides), no paid API.

## 2. When it runs

```
eligible(claim) = claim has a current code candidate whose diffs touch a .rs file
               && every review lens has recorded its blind outcome and owed look-harder
               && (value(claim) >= min_value || !claim.reversible || control(claim))
               && no hunt yet on this candidate
```

- `value` is #596's integer score (D36; formula in the #596 design, mem_a4e276044c35).
- **`min_value` (Q1):** the 80th percentile of `value` over #596's Phase A shadow claims, frozen in
  config before use. Until Phase A data exists: irreversible claims only, and no control sample.
- **Control (Q1):** after that, 1 in 10 of the claims below `min_value`
  (`sha256(task, claim) mod 10 == 0`), so we can tell whether the threshold helps.
- Rust only (Q2, matches D45).
- Not run in #596 Phase B arms, so it cannot skew D36's comparison.

## 3. One hunt

Up to 4 model calls (`Tools::ReadOnly`), each one stateless `Model::ask`, metered before the call:

1. **Available set (no call):** every evidence item on the card: note, author, cited refs.
2. **Enumerate (1 call):** up to 12 suspects `{site, mechanism, sketch, coherence 1..5}`, where
   `site` is a `crate::module::fn` or file path in the diff and `mechanism` an id from §4.
3. **Rank (pure):** §5; keep the top 3.
4. **Hunt (≤ 3 calls):** per suspect, one integration test file asserting the *correct* behaviour,
   or nothing.
5. **Run (no call):** §6.

Cost: ≤ 4 calls on the lens's meter (meter.rs ceilings) plus ≤ 6 sandbox runs. Nothing is debited
from the claim's BBP budget, because nothing goes through BBP (resolves M3).

## 4. Defect-mechanism list (Q2)

Data in the lens config, per language, Rust first. Each entry: `{id, name, look_for, keywords}`.
`keywords` tags ordinary evidence notes with mechanisms (N2).

`rs-cast` lossy casts and overflow on lengths/offsets · `rs-panic` reachable unwrap/index/slice/div
panics on external input · `rs-utf8` byte vs char boundaries, non-UTF-8 paths · `rs-path` traversal,
symlinks, TOCTOU · `rs-err` swallowed or success-mapped errors · `rs-drop` leaks or partial writes on
early return · `rs-atomic` non-atomic replace, missing fsync · `rs-conc` lock order, races ·
`rs-time` clocks, timeouts, deadline arithmetic · `rs-limit` empty/one/max/huge inputs, unbounded
allocation · `rs-unsafe` aliasing, FFI assumptions · `rs-spec` the diff contradicts the claim text,
a dependency claim or the intent.

Unknown mechanism ids are dropped by the parser.

## 5. Ranking (Q3)

Deterministic, nothing trained, no embeddings (none are wired into orch; paid ones are ruled out).

- `unavailability` = 2 when no available item names the suspect's `site` (full `module::fn` or path,
  not bare names like `new`; N3) and none is tagged with its mechanism; 1 when one matches; 0 when
  both do.
- `coherence` = the enumerating model's 1..5 rating.
- Sort by (unavailability desc, coherence desc, mechanism id, site). This keeps alien-hunt's
  β = 0.7 intent (unavailability first) without z-scores over ≤ 12 items (N1). Deterministic (D28).

Revisit with a model-rated unavailability if 20 hunts find nothing.

## 6. Running a reproducer (v1, outside BBP)

- **Where:** a scratch worktree of the candidate tree (base + diffs, as a holdout run builds it),
  with the reproducer at `tests/probe_<claim>_<n>.rs` in the crate the site belongs to. Integration
  tests reach only the public API, so suspects behind private functions are dropped at ranking (M6).
- **How:** in the existing lens sandbox (rusty_sandbox, no network): `cargo test -p <crate> --test
  probe_<claim>_<n> --no-run` first; a build failure is `error`, not a finding (resolves B1). Then run it.
- **Base check (M5):** the same probe also runs on `base`. Fails on both: `pre-existing`. Passes on
  base, fails on the candidate: `regression`, the only kind that counts as a finding. Passes on the
  candidate: nothing. A new suspect site that does not exist on base can only be `regression` or
  nothing if it builds on base; otherwise it is `new-code` and recorded as such.
- **Record:** one orch-store row per probe: task, claim, candidate, suspect, outcome
  (`error | pass | pre-existing | regression | new-code-fail`), units and tokens spent, the probe file
  hash. `rusty_orch lens alien report` prints the §8 numbers.

## 7. What v1 does not do (v2, only after §8's bar)

Posting findings to BBP. When §8 shows the lens earns it, a v2 design adds a `probe` suite so a
`regression` becomes executed counter evidence that blocks acceptance (D30). The review's B2, B3 and
M7 list what v2 must specify (per-probe digests, runs scoped to the claim's probes, reruns on new
candidates, upload path and role capability, `task_opened` digest, the exact schema list). That is a
BBP schema change and comes back to Nano as its own design.

## 8. Success bar (shadow)

Over the first 20 eligible hunts: keep and design v2 if at least 1 `regression` per 20 hunts is one
that no ordinary lens's counter evidence named (site and mechanism both unmatched, the §5 test), at
under 15% of the lens's spend on those claims. Otherwise stop, report, and revisit ranking.

## 9. Code shape (after approval), four small PRs

1. `orch-lens` port: `Question` gains `shape: Shape { Posts(Allowed), Raw }` so the enumerate and
   hunt replies (not board posts) pass the port; adapters skip `post::parse` for `Raw` (M1). Public
   API change inside orch: approval asked here.
2. `orch-lens/src/alien.rs` (pure): `eligible`, enumerate/hunt parsers, `rank`, mechanism list type,
   `Slot::Enumerate` and `Slot::Hunt(u8)`, `State.hunts` (M2). `Card` gains `current_candidate` and
   evidence `refs` from `claim_card`. Unit tests: ties, unknown mechanisms, bare-name sites, control
   sample, Rust-only.
3. `orch-store` probe rows and `rusty_orch lens alien report`.
4. Shell: scratch worktree, sandboxed build/run on candidate and base, behind `--alien`, off by
   default, live once #596's `Policy` provides `value`.

## 10. Approval asked

1. Shadow-only v1 as above, with BBP posting deferred to a v2 design.
2. Q1: `min_value` = Phase A 80th percentile; irreversible-only until then; 1-in-10 control after.
3. Q2: the Rust list in §4.
4. Q3: the §5 sort.
5. Q4 (for v1): findings stay in orch-store; the v2 BBP route returns as its own design.
6. The `Question.shape` port change in §9.1.
