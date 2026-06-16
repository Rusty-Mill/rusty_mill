# Skill Detection & Verification — `replay-skills`

> Catalog the **individual mechanics** a player executed in a replay, and answer
> "**was this skill performed?**" — by player, and within a time window.

This is the complement to the scoring crate. The design spec scopes scoring to
*decision discipline, **not** mechanics* (`replay-scoring-service-spec.md` §0),
precisely because positional/rotational signals are what kinematics support well.
`replay-skills` adds the other axis — the mechanical "what did they *do*" — under
the same honest constraint: replays carry **motion, not controller inputs**, so
every detection is inferred and carries a confidence.

Like `scoring` and `value`, it is a **pure consumer** of the
`replay_analyzer::CanonicalMatch`: no parsing, no I/O, no clock. Detection is
`detect_all(&CanonicalMatch, &SkillConfig) -> SkillReport`, deterministic and
golden-testable.

## The catalog

Twelve skills, each with a robust kinematic (or authoritative-event) signature.
Input-only mechanics (flip resets, half-flips, speedflips) are intentionally
**absent** — they cannot be distinguished from motion alone.

| Skill | Category | Source | Signature (from the canonical grid/events) |
|---|---|---|---|
| `supersonic` | Speed | kinematic | A sustained sprint at top speed; entered at `SUPERSONIC_SPEED`, held with hysteresis, min duration. |
| `aerial` | Aerial | kinematic | A ball touch with the car airborne and elevated. |
| `air_dribble` | Aerial | kinematic | ≥2 consecutive aerial touches by one player, kept up in the air. |
| `ceiling_play` | Aerial | kinematic | The car drives on / drops from near the ceiling. |
| `wall_play` | Wall | kinematic | A ball touch taken up on a side or back wall. |
| `ground_dribble` | Dribbling | kinematic | A sustained carry with the ball balanced on the roof near the ground. |
| `flick` | Dribbling | kinematic | A touch that releases a carried (low, slow) ball sharply upward. |
| `power_shot` | Striking | kinematic | A touch sending the ball fast and toward the opponent goal. |
| `redirect` | Striking | kinematic | A touch that sharply turns a fast incoming ball back toward goal. |
| `kickoff_first_touch` | Kickoff | kinematic | The first touch within the window after a kickoff. |
| `boost_steal` | Boost | kinematic | A full big-pad pickup in the opponent's half. |
| `demo` | Aggression | event | A demolition, credited to its attacker (authoritative). |

"Toward the opponent goal" and "opponent's half" use the canonical model's
per-team attack-direction sign (`Resampled::team_attack_sign`), so they are
correct for both teams.

## The verification API

`SkillReport`'s inherent methods are the query surface — the answer to *was this
skill performed*, sliceable by player and time:

```rust
report.performed(Skill::Aerial)                       // anyone, anywhere
report.performed_by(Skill::Demo, pri)                 // a specific player (PRI)
report.performed_by_name(Skill::Flick, "Schutzein")   // a specific player (name)
report.count_for(Skill::Supersonic, pri)              // how many times
report.total(Skill::Redirect)                         // lobby-wide count
report.performed_in_window(Skill::Demo, 0.0, 60.0)    // within a clip
report.performed_by_in_window(skill, pri, s, e)       // player ∧ window
report.instances_of(Skill::Aerial)                    // time-ordered occurrences
```

Each `SkillInstance` carries the time, the credited player, a `confidence`
(`0.0..=1.0`; `1.0` for event-sourced skills), and a human-readable `detail`
string of the supporting evidence.

## Proficiency profiles

Beyond presence/counts, `profiles(&SkillReport, duration_s) -> Vec<PlayerSkillProfile>`
turns the per-player roll-up into *how good / how often*: a per-skill `SkillStat`
(`count`, `per_min`, `mean_quality`) plus `total_per_min` (overall mechanical
activity). `mean_quality` is the mean detection **confidence** — a proxy that
ranks cleaner reps higher for the detectors whose confidence ramps with magnitude
(aerial height, dribble duration, redirect angle, …); event-sourced skills sit at
1.0. So two players with the same aerial count are separable by rate and quality.

`outcomes(&SkillReport, &[Event], window_s)` links skills to **goals**: per player,
how many reps fell within `window_s` before a same-team goal ("buildup
involvement"). Goals are sparse, so counts are small — read it as involvement, not
a success rate; a richer link via the value model's ΔV is a follow-up.

## CLI

```
replay-skills <file.replay> [--player <name>] [--verify <skill>]
              [--window <start_s> <end_s>] [--profile]
              [--config <cfg.json>] [--json <out.json>] [--list]
```

- default: a per-player skill table (scope to one `--player`).
- `--profile`: the proficiency view — per-player `skills/min` and per-skill
  `count`, rate, and quality.
- `--outcomes`: per player, how many skills fell in a goal buildup (within ~6 s
  before a same-team goal).
- `--verify <skill>`: prints YES/no and exits `0` if performed, `2` if not — a
  scripting gate. Combine with `--player` and/or `--window` to verify within a
  clip:
  ```
  $ replay-skills 42f2.replay --verify ceiling_play --player Schutzein
  Ceiling play: Schutzein performed YES (4 occurrences)   # exit 0
  ```
- `--list`: print the catalog.
- `--json`: write the full `SkillReport`.

## Reproducibility & calibration

Every threshold lives in a versioned `SkillConfig` (`SKILL_CONFIG_VERSION`),
never hard-coded in a detector — mirroring the scoring crate's
`SCORE_CONFIG_VERSION` discipline. The built-in defaults are **pre-calibration**:
geometry-driven, defensible guesses, not corpus-fit numbers. The detectors are
validated by synthetic-fixture unit tests (one isolated motion per skill) and a
golden digest on the validated 2v2 sample; the *thresholds* still want fitting
against a labeled corpus before the counts are trusted as absolute (the same
caveat scoring carries). The taxonomy is additive — a new skill is a detector in
`detect.rs` plus a row in `Skill::ALL`.
