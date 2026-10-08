"""Systematic one-step velocity bias by driving regime, in the car's frame.

    python -I tools/rb_tape_bot/regime_bias.py replays/batch_<stamp>... [clip_uu_s]

Runs `rb-verify --wheel-trace` on every `*_run1.jsonl`, drops ticks with a jump press or a
one-step error over `clip_uu_s` (default 5: collisions and landings, not forces), turns each
remaining tick's `dv err` (recorded minus predicted velocity change) into the car's frame with
the recorded rotation and groups it by regime: wheels touching (0 to 4), throttle sign,
boost, handbrake, steer size. For each group it prints the tick count and the mean error per
tick on each axis (forward, left, up) and a bias score, ticks x |mean|, which is the velocity
the port drifts by over the batch in that regime: a large score with a steady direction is a
missing or wrong force, a small mean with a big spread is noise.
"""
import collections
import json
import math
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


def rotate_inverse(q: dict, v: tuple) -> tuple:
    """The vector `v` expressed in the frame of the rotation `q` (world to car)."""
    x, y, z, w = -q["x"], -q["y"], -q["z"], q["w"]
    tx = 2 * (y * v[2] - z * v[1])
    ty = 2 * (z * v[0] - x * v[2])
    tz = 2 * (x * v[1] - y * v[0])
    return (
        v[0] + w * tx + (y * tz - z * ty),
        v[1] + w * ty + (z * tx - x * tz),
        v[2] + w * tz + (x * ty - y * tx),
    )


def regime(throttle: float, steer: float, flags: str, touching: int) -> tuple:
    return (
        f"{touching}w",
        "thr" + ("+" if throttle > 0.3 else "-" if throttle < -0.3 else "0"),
        "boost" if "B" in flags else "-",
        "hbrake" if "H" in flags else "-",
        "steer" + ("0" if abs(steer) < 0.1 else "half" if abs(steer) < 0.8 else "full"),
    )


def main() -> int:
    args = sys.argv[1:]
    clip = 5.0
    if args and re.fullmatch(r"[\d.]+", args[-1]):
        clip = float(args.pop())
    if not args:
        print(__doc__)
        return 2
    groups: dict = collections.defaultdict(list)
    for batch in args:
        for capture in sorted(pathlib.Path(batch).glob("*_run1.jsonl")):
            frames = [json.loads(line) for line in capture.open(encoding="utf-8")]
            t0 = frames[0]["timestamp_secs"]
            rotation_at = {round(f["timestamp_secs"] - t0, 3): f["cars"][0]["rotation"] for f in frames if f.get("cars")}
            out = subprocess.run([str(VERIFY), "--wheel-trace", str(capture), "0.5", "100"], capture_output=True, text=True).stdout
            for line in out.splitlines():
                m = LINE.match(line)
                if not m or "J" in m.group(7) or float(m.group(11)) > clip:
                    continue
                q = rotation_at.get(round(float(m.group(1)), 3))
                if q is None:
                    continue
                touching = sum(1 for _, kind in WHEELS.findall(line) if kind == "n(")
                err = rotate_inverse(q, (float(m.group(8)), float(m.group(9)), float(m.group(10))))
                groups[regime(float(m.group(2)), float(m.group(3)), m.group(7), touching)].append(err)
    rows = []
    for key, errs in groups.items():
        n = len(errs)
        mean = [sum(e[i] for e in errs) / n for i in range(3)]
        rows.append((n * math.sqrt(sum(c * c for c in mean)), n, key, mean))
    print(f"{'regime':<44}{'ticks':>7}  mean error per tick, car frame (fwd, left, up)   score")
    for score, n, key, mean in sorted(rows, reverse=True)[:int(__import__("os").environ.get("TOP", "14"))]:
        print(f"{' '.join(key):<44}{n:>7}  ({mean[0]:6.2f}, {mean[1]:6.2f}, {mean[2]:6.2f})   {score:8.0f}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
