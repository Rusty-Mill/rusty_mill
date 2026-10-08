"""The game's own run-to-run spread, against the port's error (PARITY-PLAN F).

    python -I tools/rb_tape_bot/spread_report.py replays/batch_<stamp> [name-prefix]

For every tape with two or more captures (`<name>_run<k>.jsonl`) in the batch
directory, reports

* spread: how far apart the game's own runs of the same tape end up. Each
  run's first car is sampled every 1/120 s from its first frame (so input
  start jitter counts, as it does for the port), and every pair of runs is
  compared; mean and max position distance, median over pairs;
* port: rb-verify's mean position error of each run against the port fed the
  recorded inputs; the best run is the "nearest run".

A tape is "inside the spread" when the port's best-run mean error is no more
than the larger of the spread mean and `FLOOR_UU` (the 5 uu probe bar), which
is the parity criterion for a tape the game itself does not repeat.
Scenario files are looked up in `scenarios/` then `experiments/`.
"""
import itertools
import json
import pathlib
import re
import statistics
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
VERIFY = ROOT / "target" / "release" / "rb-verify.exe"
BOT = ROOT / "tools" / "rb_tape_bot"
STEP_SECS = 1.0 / 120.0
HORIZON_SECS = 6.0
FLOOR_UU = 5.0
START_TOL_UU = 20.0
START_WINDOW = 400

Track = list[tuple[float, tuple[float, float, float]]]


def load_track(path: pathlib.Path, start: tuple[float, float, float]) -> Track:
    """(seconds since the tape's start, first car's position) per frame.

    The start is the first frame within `START_TOL_UU` of the scenario's start
    location (the closest of the first `START_WINDOW` frames when none is):
    older captures hold pre-start frames, newer ones begin on it.
    """
    frames: Track = []
    with path.open(encoding="utf-8") as lines:
        for line in lines:
            frame = json.loads(line)
            if not frame.get("cars"):
                continue
            p = frame["cars"][0]["position"]
            frames.append((float(frame["timestamp_secs"]), (p["x"], p["y"], p["z"])))
    window = frames[:START_WINDOW]
    dists = [sum((a - b) ** 2 for a, b in zip(p, start)) ** 0.5 for _, p in window]
    first = next((i for i, d in enumerate(dists) if d < START_TOL_UU), None)
    if first is None:
        first = dists.index(min(dists))
    t0 = frames[first][0]
    return [(t - t0, p) for t, p in frames[first:]]


def start_location(scenario: pathlib.Path) -> tuple[float, float, float]:
    loc = json.loads(scenario.read_text(encoding="utf-8"))["car"]["location"]
    return (loc[0], loc[1], loc[2])


def resample(track: Track, horizon: float) -> list[tuple[float, float, float]]:
    """Positions on a fixed grid by linear interpolation, up to `horizon`."""
    out: list[tuple[float, float, float]] = []
    i = 0
    n = int(min(horizon, track[-1][0]) / STEP_SECS)
    for k in range(n + 1):
        t = k * STEP_SECS
        while i + 1 < len(track) - 1 and track[i + 1][0] < t:
            i += 1
        (ta, pa), (tb, pb) = track[i], track[i + 1]
        w = 0.0 if tb <= ta else min(max((t - ta) / (tb - ta), 0.0), 1.0)
        out.append(tuple(a + (b - a) * w for a, b in zip(pa, pb)))  # type: ignore[arg-type]
    return out


def pair_error(a: list[tuple[float, float, float]], b: list[tuple[float, float, float]]) -> tuple[float, float]:
    """Mean and max distance over the common length of two sampled runs."""
    dists = [sum((x - y) ** 2 for x, y in zip(p, q)) ** 0.5 for p, q in zip(a, b)]
    return sum(dists) / len(dists), max(dists)


def spread(runs: list[list[tuple[float, float, float]]]) -> tuple[float, float]:
    """Median pairwise mean error and the largest pairwise max error."""
    pairs = [pair_error(a, b) for a, b in itertools.combinations(runs, 2)]
    return statistics.median(m for m, _ in pairs), max(x for _, x in pairs)


def scenario_path(name: str) -> pathlib.Path | None:
    for folder in ("scenarios", "experiments"):
        path = BOT / folder / f"{name}.json"
        if path.exists():
            return path
    return None


def port_mean(scenario: pathlib.Path, capture: pathlib.Path) -> float | None:
    out = subprocess.run(
        [str(VERIFY), "--scenario", str(scenario), "--against", str(capture), "--recorded-inputs"],
        capture_output=True,
        text=True,
    )
    m = re.search(r"position error: mean ([\d.]+) uu", out.stdout + out.stderr)
    return float(m.group(1)) if m else None


def tapes(batch: pathlib.Path, prefix: str) -> dict[str, list[pathlib.Path]]:
    found: dict[str, list[pathlib.Path]] = {}
    for capture in sorted(batch.glob("*_run*.jsonl")):
        m = re.fullmatch(r"(.+)_run(\d+)\.jsonl", capture.name)
        if m and m.group(1).startswith(prefix):
            found.setdefault(m.group(1), []).append(capture)
    return {name: paths for name, paths in found.items() if len(paths) >= 2}


def main() -> int:
    if len(sys.argv) < 2:
        print(__doc__)
        return 2
    batch = pathlib.Path(sys.argv[1])
    prefix = sys.argv[2] if len(sys.argv) > 2 else ""
    found = tapes(batch, prefix)
    if not found:
        print(f"no tape with two or more runs in {batch}")
        return 1

    print(f"{'tape':<22}{'runs':>5}{'spread mean':>13}{'spread max':>12}{'port best':>11}{'port mean':>11}  verdict")
    spreads: list[float] = []
    bests: list[float] = []
    inside = 0
    for name, paths in found.items():
        scenario = scenario_path(name)
        if scenario is None:
            print(f"{name:<22}{len(paths):>5}  no scenario file")
            continue
        start = start_location(scenario)
        sampled = [resample(load_track(p, start), HORIZON_SECS) for p in paths]
        s_mean, s_max = spread(sampled)
        scores = [port_mean(scenario, p) for p in paths]
        good = [s for s in scores if s is not None]
        if not good:
            print(f"{name:<22}{len(paths):>5}{s_mean:>13.1f}{s_max:>12.1f}{'-':>11}{'-':>11}  unscored")
            continue
        best, avg = min(good), sum(good) / len(good)
        ok = best <= max(s_mean, FLOOR_UU)
        inside += ok
        spreads.append(s_mean)
        bests.append(best)
        print(f"{name:<22}{len(paths):>5}{s_mean:>13.1f}{s_max:>12.1f}{best:>11.1f}{avg:>11.1f}  {'inside spread' if ok else 'OUTSIDE'}")
    if bests:
        print(
            f"\n{len(bests)} scored tapes: median spread mean {statistics.median(spreads):.1f} uu, "
            f"median port best-run mean {statistics.median(bests):.1f} uu, "
            f"{inside} inside the spread (floor {FLOOR_UU:.0f} uu)"
        )
    return 0


if __name__ == "__main__":
    sys.exit(main())
