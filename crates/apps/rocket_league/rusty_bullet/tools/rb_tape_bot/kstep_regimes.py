"""Where does the port's short-horizon error sit, by what the car was doing?

    python -I tools/rb_tape_bot/kstep_regimes.py <k> <capture.jsonl>...

Runs `rb-verify --seed-first-frame --self-kstep <capture> <k> 3000` on each
capture (the port seeded from each recorded frame, predicting `k` ticks ahead
with the recorded input) and buckets the grounded windows (no jump input, the
car under 40 uu high throughout) by throttle sign, steer size, boost,
handbrake and speed. Prints the median, 90th percentile and mean velocity error
(uu/s) per bucket with at least 25 windows, worst median first. A median
rather than a mean, because a single collision or a brake/engine switch at a
speed threshold dominates a bucket's mean. Meant for the fuzz tapes
(`gen_fuzz.py`), where it points at the controls the targeted probes missed:
on the 48 fuzz tapes of 2026-10-07 only full steer with boost under 300 uu/s
stood out (median 9.5 uu/s, one brake-to-engine switch at |forward speed| 25).
"""
import collections
import json
import pathlib
import re
import statistics
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
VERIFY = ROOT / "target" / "release" / "rb-verify.exe"
LINE = re.compile(
    r"t=\s*([\d.]+)s car=0 \| thr\s+(-?[\d.]+) str\s+(-?[\d.]+) p\s+(-?[\d.]+) y\s+(-?[\d.]+) r\s+(-?[\d.]+) (\S+) \| err vel\s+([\d.]+)"
)


def regime(throttle: float, steer: float, flags: str, speed: float) -> tuple:
    return (
        "thr" + ("+" if throttle > 0.3 else "-" if throttle < -0.3 else "0"),
        "steer" + ("0" if abs(steer) < 0.1 else "half" if abs(steer) < 0.8 else "full"),
        "boost" if "B" in flags else "-",
        "hbrake" if "H" in flags else "-",
        "spd" + ("<300" if speed < 300 else "<1000" if speed < 1000 else "<1500" if speed < 1500 else "hi"),
    )


def main() -> None:
    k = sys.argv[1]
    buckets = collections.defaultdict(list)
    for capture in sys.argv[2:]:
        frames = [json.loads(line) for line in open(capture, encoding="utf-8")]
        start = frames[0]["timestamp_secs"]
        by_tick = {round((f["timestamp_secs"] - start) * 120): f for f in frames}
        out = subprocess.run(
            [str(VERIFY), "--seed-first-frame", "--self-kstep", capture, k, "3000"],
            capture_output=True,
            text=True,
        ).stdout
        for text in out.splitlines():
            m = LINE.match(text)
            if not m or float(m.group(1)) < 0.9:
                continue
            tick = round(float(m.group(1)) * 120)
            window = [by_tick.get(j) for j in range(tick - int(k), tick + 1)]
            if any(f is None for f in window):
                continue
            cars = [f["cars"][0] for f in window]
            if any(c["input"]["jump"] or c["position"]["z"] > 40 for c in cars):
                continue
            v = cars[-1]["velocity"]
            speed = (v["x"] ** 2 + v["y"] ** 2) ** 0.5
            key = regime(float(m.group(2)), float(m.group(3)), m.group(7), speed)
            buckets[key].append(float(m.group(8)))
    print(f"grounded k={k} windows: {sum(len(v) for v in buckets.values())}")
    rows = [(key, v) for key, v in buckets.items() if len(v) >= 25]
    for key, v in sorted(rows, key=lambda kv: -statistics.median(kv[1]))[:22]:
        ordered = sorted(v)
        print(key, f"n={len(v)} median={statistics.median(v):.2f} p90={ordered[int(0.9 * len(v))]:.2f} mean={sum(v) / len(v):.2f}")


if __name__ == "__main__":
    main()
