# Handoff — RLEvalSystem after the parser cross-validation harness

Status as of 2026-06-16. Paste this into a fresh session to continue. **M1/M2/M3
are closed.** The most recent workstream — the **ballchasing stat-parity track**
(full per-player stat block + boost-pad pickup model + viewer overlays, all
validated against ballchasing ground truth) — is mostly done (see the TL;DR and
`docs/ballchasing-parity.md`). This handoff is forward-looking: the standing CI
condition, the recommended next threads, and the context/gotchas.

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
- **Ballchasing stat-parity track (mostly done) — see `docs/ballchasing-parity.md`:**
  Our per-player stat surface went from 5 fields to a full ballchasing-shaped block.
  - **`analyze::bcstats::ballchasing_stats(&CanonicalMatch)`** — pure reducer →
    boost / movement / positioning / demo aggregates (speed & boost buckets,
    air/ground, thirds/halves, behind/in-front, dist-to-ball incl. possession-split,
    most-back/forward, goals-against-while-last-defender, `time_ball_in_side`).
    Surfaced via `replay-analyzer --bc-stats <json>`. (#40, #42, #47)
  - **`analyze::boost_pads`** — boost-pad pickup model: attributes gauge gains to
    the 6 big / 28 small pads (`field::BIG/SMALL_BOOST_PADS`), tags stolen/overfill,
    and gives jitter-free collected/used. (#41)
  - **Validation gate** `tests/external_validation.rs::bcstats_agrees_with_ballchasing`
    — vs ground truth on 16 corpus replays: supersonic ρ≈0.999, dist ρ≈0.989,
    bpm ρ≈0.94, bcpm ρ≈0.92. (#46)
  - **Viewer** gained: boost-amount-above-car, playhead-driven scoreboard,
    **boost-pad pickup-map** (pads flash on collection, team-tinted), and a
    **pressure / ball-side strip**. (#43–#45)
- **Ballchasing-analyzer reverse-engineering track (2026-06-20):**
  - **Teardown spec** `docs/ballchasing-analyzer-teardown.md` — a mechanical
    "how does ballchasing compute each stat from the bytes" companion to the
    reference-vs-ours parity audit (pipeline, replay substrate, exact/inferred
    stat definitions, REST schema, the two replicated attributes ballchasing uses).
  - **Boost / powerslide / empty-header — validated against ground truth.** A
    5-replay ballchasing comparison (`docs/ballchasing-comparison.md`) drove three
    fixes: (1) **boost** uses the **gauge-delta** model (`analyze::boost_pads`) —
    `amount_collected` 0.8%, counts ~4.5%, bpm 6.4% — NOT the authoritative
    `VehiclePickup` event stream, which over-counts (a reverted regression; the raw
    `CanonicalMatch::pickups` stream is kept for viewer timing only); (2)
    **powerslide** from `bReplicatedHandbrake` is **ground-gated** to the grid
    (count 0.0% error); (3) **empty-header scoreboard** reads the network
    `PRI_TA:Match*` counters (`ActorUpdate::PriStat`) and synthesizes the roster in
    `build_canonical` when `PlayerStats[]` is empty (was 0; now exact).
    `tests/pickups_powerslide.rs`.
  - **`bc-clone` crate** — emits ballchasing's exact `GET /replays/{id}` JSON
    schema, plus **`bc-validate`** which diffs that document field-by-field
    against a real ballchasing doc (roster-paired, EXACT + 12%-CORE gates) — a
    precise regression oracle. `scripts/ballchasing_fetch.py --full-stats`
    captures the truth fixture (key-gated, never in CI).
  - **Still open** (parity-doc backlog): per-pad boost **heatmaps**, surfacing
    `bcstats` aggregates in the viewer roster, and the three `bc-clone`
    `UNIMPLEMENTED` fields (`amount_used_while_supersonic`,
    `time/percent_closest/farthest_to_ball`).
- **Corpus is ranked-2v2-only — recorded and enforced** (#39): `manifest.json`
  carries `playlist`/`team_size`/`playlist_source`; `calibrate` + `train_corpus`
  filter to `ranked-doubles` + `team_size==2` and drop any non-2v2 decode.
  Regenerate with `assets/corpus/refresh_manifest_playlist.py`.

## ℹ️ Standing condition: GitHub Actions is intentionally disabled
The owner **deliberately disabled GitHub Actions** (as of 2026-06-16); no workflow
runs have triggered repo-wide since ~10:37 UTC and none are expected to. This is
**not a blocker to fix** — it's the standing state. Consequences to internalize:
- The CI gates (the two harness checks, `cargo test --workspace`, `pytest`) are
  **not enforced remotely**. The **local gates are the source of truth** — run them
  before every merge (see Conventions below); don't wait for a green check.
- **Merging PRs without a remote check is expected**, not a workaround. PRs #32–#37
  were merged this way (each passes every check locally).
- Don't invest in new *CI-only* gates while Actions is off — they won't run. Prefer
  checks that are also runnable locally (e.g. a `--gate` binary), or wire the
  corpus/local gates the existing follow-ups describe.

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
- **Ballchasing-parity remainder (mostly token-gated):** the `contract.rs`
  **Tier-2/Tier-3 band** over the new boost/movement/positioning aggregates needs
  ballchasing's *full* per-player block, which the committed fixture doesn't carry
  — so it needs a `BALLCHASING_API_KEY` fetch (egress allows GET) to enrich the
  fixture first. Unblocked leftovers: per-pad boost **heatmaps** (the live pickup
  map already exists), and surfacing `bcstats` aggregates in the viewer roster.
  *Needs a product call:* better viewer car models. Boost economy is **already
  cross-checked** by `bcstats_agrees_with_ballchasing` (supersedes the old
  "Tier-3 boost spike" thread).
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
  **squash-merge** after the local gates pass (Actions is intentionally disabled —
  see the standing-condition section above; merge without a remote check).
- Rust gates (the source of truth, run locally): `cargo fmt --all -- --check`,
  `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`.
  Python: `cd service && pytest -q`. `recon-check` tests are heavy (subtr-actor) —
  run `--release` locally.
- Don't add a heavy dep where stdlib/serde suffices. Match the codebase's
  ports/adapters style and comment density. End commit messages with the session
  URL footer.
