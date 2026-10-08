# Detection Catalog

> A complete reference of everything this workspace can detect or derive from a
> single Rocket League `.replay`, organized from authoritative header truth up
> through inferred analysis. For each item: what it is, where it comes from, and
> which crate/module produces it.

## Provenance legend

Every row is tagged by how much to trust it:

- **authoritative** — read directly from the replay (header stats, goals, demos).
  Ground truth.
- **inferred** — derived from kinematics. Replays carry **motion, not controller
  inputs** (see `replay-scoring-service-spec.md` §0), so these are heuristic and,
  where applicable, carry a confidence. Thresholds are versioned and
  **pre-calibration** unless noted.
- **learned** — fit to an objective in-replay target (next-goal outcome).

Field constants referenced below (`src/field.rs`, unreal units): side walls
`|x|=4096`, back walls `|y|=5120`, ceiling `z=2044`, ball radius `92.75`,
supersonic `≥2200 uu/s`. These assume **standard Soccar** geometry — correct for
the competitive arenas (cosmetic reskins, identical collision). Non-standard
maps/modes (Hoops, Dropshot, …) are recognized by `field::classify_map` and
flagged (scoring → low-confidence; the viewer shows a banner; CLIs warn) rather
than mis-analyzed.

---

## 1. Match metadata & header truth — *authoritative*

Source: `src/decode` → `CanonicalMatch` / `PlayerMeta` (`src/model.rs`).

| Item | Detail |
|---|---|
| Match info | map, team size, record FPS, frame count, duration (s) |
| Team scores | final score per team |
| Per-player header stats | name, team, end-of-match score, **goals, assists, saves, shots** |
| Reproducibility | parser version (`boxcars-x.y.z`) + analyzer version, pinned into the model |

## 2. Reconstructed world state — *authoritative kinematics*

Source: `src/analyze/{reconstruct,resample,normalize}.rs`.

| Item | Detail |
|---|---|
| Ball state | position + velocity, every frame and on a fixed 30 Hz grid |
| Car state | position, velocity, boost (0–255 byte), orientation (pitch/yaw/roll) |
| Player tracks | recycled car-actors coalesced into one identity-stable track per player |
| Respawn gaps | explicit dead/respawn intervals; state is never carried across them |
| Attack-direction normalization | per-team sign so "toward opponent goal" / "own half" are comparable across teams |

## 3. Match events (T4) — *mixed*

Source: `src/analyze/events.rs` → `Event` (`src/model.rs`).

| Event | Provenance | Captured |
|---|---|---|
| **Goal** | authoritative | time, scorer, team |
| **Demo** | authoritative | time, attacker + victim (resolved to players) |
| **Kickoff** | inferred | time (ball centered at rest + cars on canonical spawns) |
| **Touch** | inferred | time, player, team (ball velocity discontinuity + nearest car within ~300 uu) |
| **Possession** | inferred | team, start/end, touch count (maximal run of same-team touches) |

## 4. Per-player aggregate features (T5) — *inferred*

Source: `src/analyze/features.rs` → `PlayerFeatures`.

Touches · boost consumed (0–100 scale) · time supersonic (s) · mean distance to
ball (uu) · team possession time (s).

---

## 5. Mechanical skills — *inferred* (`replay-skills`)

Source: `skills/src/{skill,detect,config}.rs`. Full design notes in
[`skill-detection.md`](skill-detection.md). Every detection emits a
`SkillInstance` (skill, time, player/team, **confidence 0–1**, evidence string)
and rolls up into per-player counts. Thresholds are the `SkillConfig` defaults
(`SKILL_CONFIG_VERSION = skcfg-v3`).

| Skill | Category | Granularity | Trigger | Default thresholds |
|---|---|---|---|---|
| **Supersonic** | Speed | per sprint | Sustained top speed (hysteresis-merged) | enter ≥2200 uu/s, hold ≥2100, last ≥0.5 s |
| **Aerial** | Aerial | per touch | Touch with car + ball elevated | car z ≥300, ball z ≥250 (full conf at car z 900) |
| **Air dribble** | Aerial | per run | ≥2 consecutive aerial touches, one player | gaps ≤2.0 s |
| **Ceiling play** | Aerial | per visit | Car near the ceiling | within 200 uu of ceiling (z ≥1844), ≥0.1 s |
| **Wall play** | Wall | per touch | Touch up on a side/back wall | within 150 uu of wall plane, z ≥300 |
| **Ground dribble** | Dribbling | per run | Ball balanced on roof near ground | ball z 120–350, ≤140 uu over car, carrier z <120, ≥0.75 s |
| **Flick** | Dribbling | per touch | Carried ball popped upward | incoming ≤800 uu/s, release vz ≥550 |
| **Power shot** | Striking | per touch | Fast strike toward opponent goal | ball ≥2000 uu/s, ≥60% goalward |
| **Redirect** | Striking | per touch | Sharp turn of a fast ball, goalward | incoming ≥700, angle ≥55°, result ≥1200 uu/s |
| **Kickoff first touch** | Kickoff | per kickoff | First touch after a kickoff; reaction measured from car release (GO) | ≤10 s setup window |
| **Boost steal** | Boost | per pickup | Full big-pad grab in opponent's half | →≥250 byte, gain ≥100, attack-frame y ≥2000 |
| **Demo** | Aggression | per event | Demolition credited to attacker | authoritative event |

**Verification API** (`SkillReport`, the "was this performed?" surface):
`performed` · `performed_by` / `performed_by_name` · `count_for` · `total` ·
`performed_in_window` · `performed_by_in_window` · `instances_of`. Sliceable by
player and time window; exposed on the CLI as `--verify <skill>` (exit `0` if
performed, `2` if not).

**Deliberately *not* detected** — no kinematic signature without controller
inputs: flip resets, half-flips, speedflips, wave dashes, musty/stall flicks.

---

## 6. Decision-discipline scoring — *inferred* (`replay-scoring`)

Source: `scoring/src/{roles,metrics,engine,config}.rs`. Pure
`score(match, target, config) → Report`. Config version `scfg-v3`. Scores are
heuristic and want corpus calibration before the numbers are trusted.

**Roles** (`roles.rs`): per team/frame, **1st man** (pressuring) vs **2nd man**
(support), assigned by time-to-ball with hysteresis.

**14 metrics** (`metrics.rs` / `config.rs`), each normalized to 0–100 and tagged
to a sub-score:

| Sub-score | Metrics |
|---|---|
| **1st man** | overcommit rate · goalside discipline (1st) · challenge timing · first-touch value |
| **2nd man** | support spacing · central-support fraction · double-commit rate · transition readiness |
| **General** | boost management · possession retention · ball-chase index · goalside discipline (team) · recovery speed · aerial presence |

**Report outputs** (`engine.rs` / `report.rs`):

| Output | Detail |
|---|---|
| Composite + 3 sub-scores | weighted 0–100 |
| Licence band | Unranked → Silver → Gold → Platinum → Diamond → Elite → Pacifist Master |
| Player type | Calm Controller · Diver · Stacker (nearest-centroid on 5 behavioral features) |
| Main leak | the single highest-impact metric deficit → mapped to a book chapter |
| Confidence | `ok` / `low_confidence` (too few analyzable frames, or a missing/AFK teammate) |
| Report assembly | lobby comparison table, SVG position/touch heatmaps, self-contained HTML |

## 7. Outcome value model — *learned* (`replay-value`)

Source: `value/src/{features,dataset,model,eval}.rs`. Config version
`VALUE_CONFIG_VERSION`. Learns from an **objective** target extracted from the
replay's own goals, so it needs no external corpus.

| Output | Detail |
|---|---|
| Value | `P(team scores the next goal within T seconds)` from any match state |
| Per-player ΔV | the scoring-probability swing each touch caused — an independent validator of the rubric |
| Model diagnostics | dataset rows, base rate, log-loss |

## 8. Reconstruction validation — *cross-check* (`validate`)

Source: `src/analyze/validate.rs` + `src/bin/validate.rs`. Compares our
reconstruction against ballchasing.com ground truth.

Metrics paired and scored: **time supersonic, average distance to ball, boost
collected per minute (BCPM), boost used per minute (BPM)** — reported as Spearman
+ Pearson correlation, median relative error, and player-match coverage.

---

## Reproducibility & calibration

Authoritative items (§1, §3 goals/demos) and the value model's target (§7) are
objective. Everything inferred (§3 kinematic events, §4–6) runs on **versioned,
pre-calibration thresholds** — defensible and golden-anchored, but the absolute
counts/scores should be fit against a labeled corpus before being trusted as
ground truth. Same inputs + same config versions ⇒ identical output, by design.
