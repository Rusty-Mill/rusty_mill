# Erosion calibration (card #704 slice 5)

This is the calibration that D42 requires before the erosion gate leaves report-only. It measures what rusty_mill's own merged PRs did to the two erosion shares and proposes limits. Nano sets the limits (D42, Q9). How an approved limit is recorded is OPEN_QUESTIONS Q20. Build defaults are in D110.

## Method

- **Sample.** Every merged PR of `Rusty-Mill/rusty_mill` not authored by a bot, as of 2026-11-05: 1,182 PRs. The two sets below follow Q36's interim choice:
  - The **strict set** (108 PRs) is the branches not under `claude/`, `codex/` or `import/`, as the closest available proxy for human PRs.
  - The **wide set** (1,145 PRs) is every non-bot PR except the 37 `import/` subtree imports.
- **Measurement.** Each PR's merge commit (its squash commit for older PRs) is measured against its first parent with `erosion check --base <first parent> --report-only`, from the same binary the profile runs. Every PR was measured; none errored.
- **Recorded run.** The tables below come from one fixed run: the 1,182 PRs, #10 to #1521, that `prs.sh` listed on 2026-11-05. The binary was built from the `rusty_erosion` source of commit `f407699455`, which is unchanged through this report's commit. [`erosion-calibration-2026-11-05.tsv`](erosion-calibration-2026-11-05.tsv) holds that run, one row per PR: number, branch, merge commit, the two share deltas, the function and CC deltas, unmeasured files by extension and an error flag. The tables can be checked against it without measuring again: `python3 -I crates/tools/rusty_erosion/calibrate/summarize.py report docs/projects/let-em-loose/erosion-calibration-2026-11-05.tsv`.
- **Reproducing it.** From the repository root, in a full-history clone (`git fetch --unshallow origin main`):

  ```sh
  cargo build --release -p rusty_erosion
  BIN="$(cargo metadata --format-version 1 --no-deps | jq -r .target_directory)/release/erosion"
  C=crates/tools/rusty_erosion/calibrate
  OUT="$PWD/erosion-calibration" && mkdir -p "$OUT"
  "$C/prs.sh" | tr '\t' ' ' | xargs -P 4 -L 1 sh -c \
    "$C/measure.sh"' "$0" "$1" "$2" "$3" "$4" "$5"' "$BIN" "$PWD" "$OUT"
  python3 -I "$C/summarize.py" rows "$OUT" > rows.tsv
  python3 -I "$C/summarize.py" report rows.tsv
  ```

  A run takes about 15 seconds per PR on 4 cores. `prs.sh` lists every PR merged when it runs, so a later run includes newer PRs and won't match the tables exactly. To reproduce the recorded run, keep only PRs up to #1521 and build `rusty_erosion` at `f407699455`.

## Results

Deltas are in basis points of a repo-wide share. A limit fails a PR when a delta is greater than the limit, as `erosion check` does.

### Strict set: 108 PRs

| Metric | p50 | p90 | p95 | p99 | max | PRs that raise it |
| --- | --- | --- | --- | --- | --- | --- |
| concentration_bp | 0 | 0 | 1 | 1 | 2 | 7 |
| duplication_bp | 0 | 0 | 0 | 1 | 2 | 2 |

| Limit pair (bp) | PRs it would fail |
| --- | --- |
| 0 / 0 | 8 (7.4%) |
| 1 / 1 | 2 (1.9%) |
| 2 / 2 | 0 |

- **Unmeasured (D45).** 30 PRs changed 96 files erosion can't measure: py 52, sh 15, ps1 11, ts 8, jsx 5 and js 2.
- **Gaming signal.** 6 PRs lowered concentration while adding functions or CC.

### Wide set: 1,145 PRs

| Metric | p50 | p90 | p95 | p99 | max | PRs that raise it |
| --- | --- | --- | --- | --- | --- | --- |
| concentration_bp | 0 | 0 | 1 | 5 | 38 | 76 |
| duplication_bp | 0 | 0 | 1 | 8 | 420 | 64 |

| Limit pair (bp) | PRs it would fail |
| --- | --- |
| 0 / 0 | 132 (11.5%) |
| 1 / 1 | 81 (7.1%) |
| 2 / 2 | 49 (4.3%) |
| 5 / 5 | 25 (2.2%) |
| 10 / 10 | 13 (1.1%) |
| 25 / 25 | 6 (0.5%) |

- **Unmeasured (D45).** 236 PRs changed 5,127 files erosion can't measure: ts 3,420, tsx 855, py 484, sh 191, jsx 55 and js 52. Most of these are the web front ends.
- **Gaming signal.** 147 PRs lowered concentration while adding functions or CC.
- **Outliers.** The largest deltas come from the 2026-08 repo consolidation (#16, #18) and the 2026-09 Nexus migration (#159), not from feature work.

At #1521's merge commit `0d64d87b`, the repo stands at 2,367 bp concentration and 3,021 bp duplication.

## What the numbers say

1. **The p90 limit is 0 / 0.** Fewer than 10% of PRs move either share at all, so p90 is zero in both sets. A 0 / 0 limit fails any PR that raises a share by a single basis point: 7.4% of the strict set and 11.5% of the wide set.
2. **The shares are repo-wide, so one PR barely moves them.** A delta is the exact difference of the two shares, truncated toward zero, so a reported delta of 1 means the share really rose by at least 1 bp. Most PRs raise a share by less than that and report 0. The gate then sees only PRs that add a lot of complex or duplicated Rust, which is the intended signal.
3. **Unmeasured code is mostly TypeScript and Python.** Agent PRs that touch the web apps carry it (D45 reports it and doesn't fail it). In the strict set it is mostly Python and shell.
4. **The gaming signal is weak evidence.** A PR that adds many small functions lowers concentration honestly, so a count of PRs isn't proof of gaming. The signal is worth watching per claim (D43's worst functions), not as a limit.

## Proposal

The p90 rule from the card gives **0 bp concentration and 0 bp duplication**. The rule is per metric, but the gate fails a PR on either metric, so as a pair 0 / 0 fails more than 10% of the wide set (11.5%), though 7.4% of the strict set. I recommend **1 bp / 1 bp**, the smallest pair that fails fewer than 10% of PRs in both sets: 2 of the 108 strict-set PRs (1.9%) and 7.1% of the wide set. It lets a PR raise a share by less than 2 bp. 0 / 0 is the stricter option, and 2 / 2, which fails no strict-set PR, is the looser one.

D42 makes this Nano's choice. Until Nano picks limits, the profile stays report-only. Under D88, once the time box ends, a new report-only claims task can no longer open.
