# `bc-clone` — a ballchasing.com analyzer clone + validator

Reverse-engineers ballchasing.com's replay analyzer two ways:

1. **`bc-clone`** — decode a `.replay` and emit a **ballchasing-shaped stats
   document**: the exact `GET /replays/{id}` JSON schema (`blue`/`orange` sides,
   each with team `stats` and a `players[]` array; every player carries the five
   nested groups `core` / `boost` / `movement` / `positioning` / `demo` under the
   field names ballchasing uses).

   ```sh
   cargo run -p bc-clone --bin bc-clone -- game.replay --out doc.json
   ```

2. **`bc-validate`** — the **exact-match validator**: diff our document against a
   real ballchasing document, field by field, and gate on agreement.

   ```sh
   # capture the real doc (manual, key-gated, never in CI):
   export BALLCHASING_API_KEY=…
   python scripts/ballchasing_fetch.py game.replay --full-stats --out truth.json
   # then diff:
   cargo run -p bc-clone --bin bc-validate -- game.replay truth.json
   ```

   It pairs rosters by `(side, name)` (tolerant of ballchasing's accent-stripping,
   via the workspace `roster_match`), prints a per-field agreement table
   (`n`, median relative error, max absolute error), and **exits non-zero** if any
   gated field fails:
   - **exact** fields (goals, score, saves/shots/assists, every `count_*`, demo,
     `goals_against_while_last_defender`) must match to the unit;
   - **core** continuous fields (the authoritative pad model, speed buckets,
     thirds/halves, distances) must agree within 12 % median relative error.

   That makes ballchasing a precise regression oracle for the analyzer.

## Faithfulness

This reuses the workspace analyzer (`replay-analyzer`: decode → reconstruct →
`bcstats`) as its engine — exactly as ballchasing reuses an external parser
(rattletrap) under its own analyzer. The "clone" here is the **output schema and
stat semantics**, reproduced field-for-field, not a from-scratch second
reconstruction (that independent check already exists as the `recon-check`
crate). The mechanical spec it implements is
[`docs/ballchasing-analyzer-teardown.md`] *(private repo)*.

## Not yet computed

Three ballchasing fields are emitted as `0.0` (and excluded from the validator):
`amount_used_while_supersonic`, `time/percent_closest_to_ball`,
`time/percent_farthest_from_ball`. They're tracked in
[`docs/ballchasing-parity.md`] *(private repo)*. Everything else —
including the authoritative boost-pad pickups and powerslide — is real.
