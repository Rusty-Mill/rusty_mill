"""Per-tick readings of the settled brake probes (brake_ring2_*, brake_sat2_*).

The car sits settled at rest, gets its speed from a short known throttle, then coasts or brakes with no other input.
Rings: after the speed peaks, each tick should multiply the forward speed by the same negative factor (model -0.2638 at
K = 113.74). Prints the peak speed and the per-tick ratio v(n+1)/v(n) for n = 0..4,
mean over the runs, plus the same pooled over tapes.
Saturated: after the run-up, prints the speed loss per tick as a deceleration (uu/s^2) around the full-brake ticks
(throttle -1 against the motion) and the coasting ticks that follow.

Usage: ``python -I tools/rb_tape_bot/brake_probe_checks.py replays/batch_<stamp>``.
"""
from __future__ import annotations

import json
import math
import sys
from pathlib import Path
from statistics import mean, pstdev


DT = 1.0 / 120.0


def axes(q: dict) -> tuple[tuple[float, float, float], tuple[float, float, float]]:
    """World-space forward (+X) and up (+Z) axes of unit quaternion ``q`` (x, y, z, w)."""
    x, y, z, w = q["x"], q["y"], q["z"], q["w"]
    fwd = (1 - 2 * (y * y + z * z), 2 * (x * y + w * z), 2 * (x * z - w * y))
    up = (2 * (x * z + w * y), 2 * (y * z - w * x), 1 - 2 * (x * x + y * y))
    return fwd, up


def load(path: Path) -> list[dict]:
    out = []
    for line in path.open(encoding="utf-8"):
        d = json.loads(line)
        if d.get("cars") and d["cars"][0].get("input"):
            out.append(d["cars"][0])
    return out


def fwd_speed(c: dict) -> float:
    f, _ = axes(c["rotation"])
    v = c["velocity"]
    return v["x"] * f[0] + v["y"] * f[1] + v["z"] * f[2]


def runs(batch: Path, stem: str) -> list[list[dict]]:
    return [load(p) for p in sorted(batch.glob(f"{stem}_run*.jsonl")) if not p.name.endswith(".pads.jsonl")]


def first_input(fr: list[dict]) -> int | None:
    return next((i for i, c in enumerate(fr) if c["input"]["throttle"] != 0), None)


def rings(batch: Path) -> None:
    print("Rings: peak speed, then per-tick ratio v(n+1)/v(n), n = 0..4 (mean over runs); model -0.2638")
    print("tape               runs  peak (uu/s)   n=0       n=1       n=2       n=3       n=4")
    pooled: list[float] = []
    for tag in ("f", "r"):
        for label in ("6", "13", "20"):
            rows, peaks = [], []
            for fr in runs(batch, f"brake_ring2_{tag}{label}"):
                s = first_input(fr)
                if s is None or s + 14 >= len(fr):
                    continue
                series = [fwd_speed(c) for c in fr[s : s + 14]]
                p = max(range(len(series)), key=lambda i: abs(series[i]))
                if p + 6 >= len(series):
                    continue
                peaks.append(series[p])
                rows.append([series[p + n + 1] / series[p + n] for n in range(5)])
            if not rows:
                print(f"brake_ring2_{tag}{label}  no usable runs")
                continue
            m = [mean(r[n] for r in rows) for n in range(5)]
            pooled.append(m[0])
            print(f"brake_ring2_{tag}{label:2s}      {len(rows)}   {mean(peaks):+8.2f}   " + "  ".join(f"{x:+.4f}" for x in m))
    if pooled:
        print(f"first-tick ratio over the six tapes: mean {mean(pooled):+.4f}  sd {pstdev(pooled):.4f}  -> K = {90 * (1 - mean(pooled)):.2f}")


def sats(batch: Path) -> None:
    print("\nSaturated brake: speed loss per tick as a deceleration (uu/s^2) from the first throttle tick on, mean over runs")
    for stem, label in (("brake_sat2_200_rev", "run-up 15 ticks, then -1 x3"), ("brake_sat2_400_rev", "run-up 30 ticks, then -1 x3"),
                        ("brake_sat2_200_coast", "run-up 15 ticks, then coast")):
        rows = []
        for fr in runs(batch, stem):
            s = first_input(fr)
            if s is None or s + 26 >= len(fr):
                continue
            series = [fwd_speed(c) for c in fr[s : s + 26]]
            rows.append([(series[n + 1] - series[n]) / DT for n in range(25)])
        if rows:
            m = [mean(r[n] for r in rows) for n in range(25)]
            print(f"{stem} ({label}), {len(rows)} runs")
            print("   tick: " + " ".join(f"{n:6d}" for n in range(8, 25)))
            print("   decel " + " ".join(f"{m[n]:6.0f}" for n in range(8, 25)))


def main() -> None:
    batch = Path(sys.argv[1])
    rings(batch)
    sats(batch)


if __name__ == "__main__":
    main()
