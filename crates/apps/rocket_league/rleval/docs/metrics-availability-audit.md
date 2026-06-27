# Metrics availability audit — available vs. shown vs. ballchasing

Status as of 2026-06-27. A three-way audit, taken from the **current source**
(not just the older parity docs), of every statistic/metric in this workspace:

1. **Available** — does the code compute it / carry it at all?
2. **Shown** — do we surface it to a user, and on which surface?
3. **Ballchasing** — does ballchasing.com expose it?

**"Shown" surfaces:**

- **App** — the unified web UI (`app/src/ui.rs`).
- **Viewer** — the 3D replay viewer HUD / roster / overlays (`viewer/`).
- **Score** — the decision-discipline HTML report (`scoring/src/render.rs`,
  `report.rs`, `lobby.rs`).
- **BC** — the ballchasing-style dashboard we render (`bc-clone/src/html.rs`).

The key structural fact: **ballchasing's entire stat surface is something we
both compute *and* render** — the `bc-clone` crate emits the exact
`GET /replays/{id}` schema and `bc-clone/src/html.rs` renders it as a full
8-tab dashboard. So vs. ballchasing the content gap is tiny (rank/MMR + one
approximate boost field). The real gap is *internal*: our own primary surfaces
(app Overview, 3D viewer roster) expose only a slice of what we compute, and our
unique IP layers (skills, decision-discipline scoring, value/impact) — which
ballchasing has **nothing** comparable to — are only partially surfaced.

---

## Ballchasing-parity stats (the `bc-clone` surface)

We compute ≈85 per-player fields across five groups plus team/ball aggregates.

### Core (scoreboard)

| Stat | Available | We show | Ballchasing |
|---|---|---|---|
| goals, assists, saves, shots, score | ✅ header truth | Viewer (G/A/Sv), BC | ✅ |
| shooting_percentage | ✅ | BC | ✅ |
| shots_against, goals_against | ✅ recomputed | BC | ✅ |
| mvp | ✅ | BC | ✅ |

We recompute saves/shots/assists as **Tier-2 advisory**; header values stay
Tier-1 truth (`scoring/src/contract.rs`).

### Boost (32 fields)

| Stat | Available | We show | Ballchasing |
|---|---|---|---|
| bpm, bcpm | ✅ | BC | ✅ |
| avg_amount | ✅ | Viewer (live bar), BC | ✅ |
| time/percent zero_boost, full_boost | ✅ | BC | ✅ |
| boost quartile histogram (0-25 … 75-100, time+%) | ✅ | BC | ✅ |
| amount_collected / stolen (× big/small) | ✅ | BC | ✅ |
| count_collected / stolen (× big/small) | ✅ | BC | ✅ |
| amount_overfill, overfill_stolen | ✅ | BC | ✅ |
| amount_used_while_supersonic | ⚠️ approximate (flagged UNIMPLEMENTED) | BC (marked) | ✅ |
| per-pad pickup map (spatial) | ✅ `PadPickup` stream | BC, Viewer (pad flashes) | ✅ |

### Movement (15 fields)

| Stat | Available | We show | Ballchasing |
|---|---|---|---|
| avg_speed, avg_speed_percentage | ✅ | BC; Viewer (live kph only) | ✅ |
| total_distance | ✅ | BC | ✅ |
| speed buckets: slow / boost / supersonic (time+%) | ✅ | BC, Viewer (bucket bar) | ✅ |
| altitude: ground / low_air / high_air (time+%) | ✅ z-bands approximate | BC | ✅ |
| time/count/avg_duration powerslide | ✅ | BC | ✅ |

### Positioning (27 fields)

| Stat | Available | We show | Ballchasing |
|---|---|---|---|
| avg_distance_to_ball (+ possession / no-possession split) | ✅ | BC | ✅ |
| avg_distance_to_mates | ✅ | BC | ✅ |
| thirds: def / neutral / off (time+%) | ✅ | BC, Viewer (thirds bar) | ✅ |
| halves: def / off (time+%) | ✅ | BC | ✅ |
| behind / in-front of ball (time+%) | ✅ | BC | ✅ |
| most_back / most_forward (time+%) | ✅ | BC, Viewer (most-back %) | ✅ |
| closest / farthest to ball (time+%) | ✅ | BC | ✅ |
| goals_against_while_last_defender | ✅ | BC | ✅ |

### Demo

| Stat | Available | We show | Ballchasing |
|---|---|---|---|
| inflicted, taken | ✅ | Viewer (ticker), BC | ✅ |

### Team / ball aggregates

| Stat | Available | We show | Ballchasing |
|---|---|---|---|
| possession_time / possession % | ✅ | Viewer, BC | ✅ |
| pressure (ball-in-side %) / time_in_side | ✅ | Viewer (pressure curve), BC | ✅ |
| team-level stat rollups | ✅ | BC | ✅ |
| ball heatmap | ✅ | BC, Viewer (minimap) | ✅ |

### Identity / camera / metadata

| Item | Available | We show | Ballchasing |
|---|---|---|---|
| name, team, map, mode/team_size, duration, final score | ✅ | App, Viewer, Score, BC | ✅ |
| car_id / car_name | ✅ | BC | ✅ |
| camera profile (FOV, dist, height, angle, stiffness, swivel, transition) + steering sensitivity | ✅ | BC | ✅ |
| full loadout / cosmetics (decal, wheels, boost, trail, paint…) | ✅ decoded, **not surfaced** | — | partial |
| ping / net quality / join-leave | ✅ authoritative | — | partial |
| competitive rank / MMR | ❌ **not in replay** (external API only) | — | ✅ (RL/tracker API) |

---

## Metrics WE have that ballchasing does NOT (our IP layers)

No ballchasing equivalent exists for any of these. They are the
differentiators — and most are **under-shown**.

| Layer | Metrics | Available | We show | BC has |
|---|---|---|---|---|
| **Skills (D)** — 13 mechanical detectors | aerial, air dribble, ceiling play, wall play, ground dribble, flick, power shot, redirect, kickoff first touch, boost steal, demo, supersonic, double-touch — each with count / timing / verify API | ✅ | App (Skills tab: per-skill counts, /min), Viewer (skill ticker) | ❌ |
| **Scoring (E)** — decision-discipline | composite score, licence band, player-type, sub-scores (1st/2nd/general), main leak→chapter, + 14 sub-metrics: overcommit, goalside discipline (1st & team), support spacing, central support, double-commit, ball-chase index, challenge timing, first-touch value, transition readiness, recovery speed, aerial presence, possession retention, boost management | ✅ | App (Overview: composite/licence/type/leak), Score (full breakdown + heatmaps) | ❌ |
| **Value (F)** — impact model | per-touch ΔV, total/mean ΔV, win-probability curve, momentum, goals-above-expected, two-track reconciliation | ✅ | App (Impact tab: touches/total/mean ΔV), Viewer (win-prob + pressure curves, per-touch ΔV chips) | ❌ |
| **Events (C)** — touch analytics | touches (count/location/height/type), possessions, kickoff role & outcome, first touch, duels/50s, passes/pre-assists, dribbles, centers | ✅ (mostly) | Viewer (ticker), partial in BC | partial |

---

## Gaps — computed but never shown

1. **The 3D viewer roster is thin.** We compute the full 85-field
   ballchasing block per player, but the roster shows only goals/assists/saves +
   live speed + live boost, with an optional expandable panel adding just a boost
   sparkline, speed-bucket bar, thirds bar, and most-back %. Distances, halves,
   powerslide, boost economy, demo counts, behind-ball, etc. never reach the
   viewer.
2. **The app Overview is a 6-column summary** (composite / licence / type /
   skills-per-min / ΔV / main-leak). None of the 85 ballchasing-class stats
   appear in the app's own tabs — they live only in the separate BC dashboard.
3. **`amount_used_while_supersonic`** is the one ballchasing field we compute
   only approximately (flagged `UNIMPLEMENTED` — reconstruction-noisy; the
   validator skips its exact comparison).
4. **Loadout cosmetics, ping / net-quality, lifecycle** are decoded /
   authoritative but surfaced nowhere.
5. **Rank / MMR** is the only thing ballchasing shows that we *cannot* derive —
   absent from the replay file, must come from an external API.

---

## Bottom line

- **Vs. ballchasing:** near-full computation parity (≈85 per-player fields +
  team/ball) and we render all of it in the BC dashboard. The only true content
  gap is rank/MMR (external) and one approximate boost field.
- **Vs. ourselves:** our primary surfaces (app Overview, 3D viewer roster)
  expose a small slice of what we compute, while our unique IP — skills,
  decision-discipline scoring, value/impact, the stuff ballchasing has no
  equivalent for — is only partially surfaced.
