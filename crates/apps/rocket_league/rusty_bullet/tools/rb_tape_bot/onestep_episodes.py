"""Sustained one-step error episodes across a batch, grouped by what the car was doing.

    python -I tools/rb_tape_bot/onestep_episodes.py replays/batch_<stamp>... [min_ticks] [min_err]

Runs `rb-verify --wheel-trace` on every `*_run1.jsonl`, and finds runs of at least
`min_ticks` (default 8) consecutive ticks whose one-step velocity error is over `min_err`
(default 1.5 uu/s), in the part of the tape after the state-set transient. A steady
error that long is a missing or wrong force, not a collision. Each episode is labelled by
the number of wheels touching, the throttle sign, boost, handbrake and steer, and the
labels are ranked by the velocity error they accumulate (the sum of the per-tick errors),
which is what grows into a position error.
"""
import collections
import pathlib
import re
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
VERIFY = ROOT / "target" / "release" / "rb-verify.exe"
LINE = re.compile(
    r"t=\s*([\d.]+)s \| thr\s+(-?[\d.]+) str\s+(-?[\d.]+) p\s+(-?[\d.]+) y\s+(-?[\d.]+) r\s+(-?[\d.]+) (\S+) .*?"
    r"dv err \(\s*(-?[\d.]+),\s*(-?[\d.]+),\s*(-?[\d.]+)\) \|\s*([\d.]+)\|"
)
WHEELS = re.compile(r"(FR|FL|BR|BL) (n\(|-)")


def label(throttle: float, steer: float, flags: str, touching: int) -> tuple:
    return (
        f"{touching} wheels",
        "thr" + ("+" if throttle > 0.3 else "-" if throttle < -0.3 else "0"),
        "boost" if "B" in flags else "-",
        "hbrake" if "H" in flags else "-",
        "steer" + ("0" if abs(steer) < 0.1 else "half" if abs(steer) < 0.8 else "full"),
    )


def episodes(capture: pathlib.Path, min_ticks: int, min_err: float):
    out = subprocess.run([str(VERIFY), "--wheel-trace", str(capture), "0.5", "100"], capture_output=True, text=True).stdout
    current: list[tuple] = []
    for line in out.splitlines():
        m = LINE.match(line)
        if not m:
            continue
        touching = sum(1 for _, kind in WHEELS.findall(line) if kind == "n(")
        row = (float(m.group(1)), float(m.group(11)), label(float(m.group(2)), float(m.group(3)), m.group(7), touching), "jump" in m.group(7))
        flags = m.group(7)
        if row[1] > min_err and "J" not in flags:
            current.append(row)
            continue
        if len(current) >= min_ticks:
            yield current
        current = []
    if len(current) >= min_ticks:
        yield current


def main() -> int:
    args = sys.argv[1:]
    min_ticks, min_err = 8, 1.5
    nums = [a for a in args if re.fullmatch(r"[\d.]+", a)]
    batches = [a for a in args if a not in nums]
    if nums:
        min_ticks = int(float(nums[0]))
    if len(nums) > 1:
        min_err = float(nums[1])
    if not batches:
        print(__doc__)
        return 2
    totals: dict[tuple, float] = collections.defaultdict(float)
    counts: dict[tuple, int] = collections.defaultdict(int)
    tapes: dict[tuple, set] = collections.defaultdict(set)
    for batch in batches:
        for capture in sorted(pathlib.Path(batch).glob("*_run1.jsonl")):
            for ep in episodes(capture, min_ticks, min_err):
                key = max(set(r[2] for r in ep), key=lambda k: sum(1 for r in ep if r[2] == k))
                totals[key] += sum(r[1] for r in ep)
                counts[key] += 1
                tapes[key].add(capture.name[:-11])
    print(f"{'regime':<58}{'episodes':>9}{'tapes':>7}{'accumulated uu/s':>18}")
    for key, total in sorted(totals.items(), key=lambda kv: -kv[1])[:16]:
        print(f"{' '.join(key):<58}{counts[key]:>9}{len(tapes[key]):>7}{total:>18.0f}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
