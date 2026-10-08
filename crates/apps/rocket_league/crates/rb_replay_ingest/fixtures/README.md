# Fixtures

## `subtr-actor-sample.replay`

A real Rocket League replay file, used here purely to integration-test the
parsing pipeline (`boxcars` + `subtr-actor` → `PhysicsFrame`) end-to-end.

- **Source**: [`rlrml/subtr-actor`](https://github.com/rlrml/subtr-actor)'s
  own `assets/problematic-private-duel-2026-03-20.replay`, which that
  project publishes and links to from its own README as a public demo/test
  fixture (served via `raw.githubusercontent.com` for their live stats-player
  demo).
- **Why it's here**: `subtr-actor`'s `ReplayData`/`FrameData`/`BallData`/
  `PlayerData` types have private constructors — there is no way to build a
  parsed-replay result by hand for a unit test. Exercising the real
  `boxcars` → `subtr_actor::ReplayDataCollector` → our `PhysicsFrame`
  conversion path requires an actual `.replay` file. This one is small
  enough to vendor and lets that integration test run offline and
  reproducibly in CI.
- **What it does *not* satisfy**: `RB-VERIFY-001`'s acceptance criteria
  call for verification against "a real replay file" from the *owner's own
  match history" with a manually cross-checked timestamp. This fixture is a
  third party's replay, used only to prove the pipeline runs correctly on
  real replay bytes (parses without error, produces sane ball/car
  positions) — not as the ground-truth data point that acceptance
  criterion asks for. That still requires the owner supplying their own
  replay file, which this environment doesn't have access to.
