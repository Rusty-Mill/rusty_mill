# Handoff — RLEvalSystem after the parser cross-validation harness

Status as of 2026-06-17. Paste this into a fresh session to continue. The last
big workstream — the parser cross-validation harness (spec §13) — is **done**
and merged; **M3 is fully closed**. This handoff is forward-looking: the blocker,
the recommended next thread, and the context/gotchas so you don't re-derive them.

## TL;DR of what exists now
- **Rust workspace** (root crate `replay-analyzer` + `scoring/`, `skills/`,
  `value/`, `viewer/`, `recon-check/`) decodes `.replay` → canonical match model →
  scoring/skills/value/viewer. **Python FastAPI `service/`** is the web/DB/monetization
  layer (M2). M1, M2, and M3 are complete.
- **Parser cross-validation harness (M3 §13) — complete:**
  - **Phase 1** — ballchasing.com **header** cross-check. `scoring/src/contract.rs`
    (`cross_check(&CanonicalMatch, &BallchasingReplay)`), bin `scoring/src/bin/contract.rs`,
    offline CI test `scoring/tests/ballchasing_contract.rs` on sanitized real
    fixtures, key-gated fetch shim `scripts/ballchasing_fetch.py`. Tiers: T1 exact
    (goals/score/roster/map/team_size), T2 advisory (saves/shots/assists), T3
    deferred (boost economy). PRs #32.
  - **Phase 2** — independent **reconstruction** cross-check. `recon-check/` crate:
    compares our `build_canonical` against **subtr-actor** (v1.0.2, pins our exact
    `boxcars 0.11.3`) frame-by-frame on a shared 30 Hz grid. Tier R1 gates roster +
    coverage + ball position (median ≤ 60 uu AND ≥ 80% of frames within 200 uu —
    an agree-rate gate, not p95, to tolerate goal-celebration windows); Tier R2
    reports per-player car position + boost. Offline CI test
    `recon-check/tests/recon_contract.rs`. PRs #33–#35.
  - Full design + measured numbers: `docs/parser-cross-validation-harness.md`.
    Backlog: `docs/backlog.md` (M3 section).

## ⛔ Top blocker (fix first): CI / GitHub Actions is down
No workflow runs have triggered repo-wide since **2026-06-16 ~10:37 UTC** — Actions
appears **quota-exhausted or disabled**. So the two harness CI gates (and the whole
`cargo test --workspace` / `pytest` suite) are **not actually enforced**. PRs
#32–#35 were merged without a green check (per the owner's direction, since the
change passes every check locally). **The fix is owner-side** (GitHub billing /
Actions settings) — you can confirm the symptom via the Actions API
(`list_workflow_runs` shows nothing since 10:37 UTC) but cannot flip billing. Raise
this before building more CI gates, since none of them run today.

## ⭐ Recommended next thread: corpus-wide recon-check
`recon-check` currently *passes* on `42f2`/`419a`. Its real value is as a
**bug-finder** — run it across the 180-replay corpus to either confirm our
reconstruction agrees with an independent decoder everywhere, or surface specific
replays where it doesn't (= real reconstruction bugs).
- The corpus `.replay` files are **gitignored**; fetch by manifest id with
  `assets/corpus/refresh_corpus_replays.py` (needs `BALLCHASING_API_KEY`; GET works).
- Build a batch runner (new bin in `recon-check`, or a small script) that calls
  `recon_check::cross_check_replay(bytes, id)` per corpus replay and aggregates R1
  pass/fail + median/agree-rate distributions, printing the offenders. Pattern it on
  the existing **local/with-corpus gate** `reconcile` (it's not a no-corpus CI gate).
- Run in **`--release`** (subtr-actor is slow in debug). Expect a few legitimate
  failures (own-goal replays, non-standard maps); **investigate them, don't just
  loosen thresholds**.

## Smaller threads (pick per appetite)
- **recon-check event-timing:** subtr-actor also exposes goal/demo/touch events;
  add a Tier-R1 cross-check of **goal/demo timestamps** (currently positions only).
  ~½ day, high cohesion.
- **Phase 1 Tier-3 boost spike:** map our `boost_used` (0–100 integral) to
  ballchasing `bpm`/`amount_collected` so boost economy can be cross-checked.
- **M2 prod standup:** provision managed Postgres / S3 bucket+IAM / Redis; swap a
  KMS/AES-GCM cipher behind the existing `Cipher` port; real auth (replace the
  `X-Account-Email` dev stub); per-player PDF scoping.
- **Value learner / skill thresholds:** heavier value model (GBM/NN; VAL AUC ≈0.71);
  calibrate `replay-skills` thresholds against the corpus.

## Critical context & gotchas (don't re-derive)
- **Egress:** `ballchasing.com` is allowlisted for **GET**, but the egress proxy
  **caps POST request bodies (~<64 KB)** — uploading a ~1.3 MB replay returns 413.
  That's why Phase 1's real fixture is a **corpus replay fetched by id**
  (`990e4485-…`), not the `42f2` sample. To pair the exact `42f2`, the owner must
  relax the egress body limit, then re-run `scripts/ballchasing_fetch.py`.
  `crates.io` is reachable (cargo builds fine).
- **ballchasing key:** from env `BALLCHASING_API_KEY` **only** — never echo, never
  commit. The key used in the prior session was pasted in chat and **must be
  rotated**.
- **recon-check independence:** it shares `boxcars` (the parse layer), so it is
  *reconstruction-layer* independence only — a `boxcars` decode bug stays invisible
  to it (Phase 1's ballchasing check covers that boundary for header facts).
- **subtr-actor usage:** `NDArrayCollector::<f32>::from_strings(&["BallRigidBody",
  "CurrentTime"], &["PlayerRigidBody","PlayerBoost"])` + `FrameRateDecorator::
  new_from_fps(hz, &mut c).process_replay(&replay)` → `get_meta_and_ndarray()`.
  Columns: 13 global (ball pos/rot/lin-vel/ang-vel, then time) + 13 per player
  (pos/rot/lin-vel/ang-vel, then boost), players in `meta.replay_meta.player_order()`.
- **Fixtures:** Phase 1 — `scoring/tests/fixtures/990e4485.{canonical,ballchasing,
  ballchasing.drift}.json` (sanitized: uploader + player ids stripped). Phase 2 —
  committed `assets/replays/{42f2,419a}.replay`.
- **Git/commit-verification quirk:** GitHub's squash-merge commit has committer
  `noreply@github.com`; the stop-hook flags it if it's your branch tip *ahead of
  upstream*. **Don't rewrite it** (it's on `main`) — instead base new work on
  `origin/main` and/or reset your local tip to upstream. Git identity is already
  `noreply@anthropic.com` / `Claude`.

## Conventions
- Develop on the session's **designated branch**; small PRs to `main`,
  **squash-merge** after CI is green (CI is currently down — prior slices merged
  without green per owner direction; confirm with the owner).
- Rust gates (CI parity): `cargo fmt --all -- --check`,
  `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`.
  Python: `cd service && pytest -q`. `recon-check` tests are heavy (subtr-actor) —
  run `--release` locally.
- Don't add a heavy dep where stdlib/serde suffices. Match the codebase's
  ports/adapters style and comment density. End commit messages with the session
  URL footer.
