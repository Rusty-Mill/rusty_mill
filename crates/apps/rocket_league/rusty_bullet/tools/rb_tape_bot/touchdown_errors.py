"""One-step velocity error summed over the first ticks after a touchdown, in the car's frame.

    python -I tools/rb_tape_bot/touchdown_errors.py replays/batch_<stamp>... [window_ticks]

A touchdown is a tick with at least one wheel touching after a tick with none (no jump
pressed). For each one the one-step velocity errors (recorded minus predicted) of that tick and
the next `window_ticks - 1` (default 4) are summed in the car's frame and listed with the
landing's wheel set and the car's vertical speed. A mean far from zero over many landings is a
missing or wrong landing force; the spread says how much is chaos.
"""
import json
import math
import pathlib
import re
import statistics
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


def main() -> int:
    args = sys.argv[1:]
    window = 4
    if args and re.fullmatch(r"\d+", args[-1]):
        window = int(args.pop())
    if not args:
        print(__doc__)
        return 2
    events = []
    for batch in args:
        for capture in sorted(pathlib.Path(batch).glob("*_run1.jsonl")):
            frames = [json.loads(line) for line in capture.open(encoding="utf-8")]
            t0 = frames[0]["timestamp_secs"]
            by_time = {round(f["timestamp_secs"] - t0, 3): f["cars"][0] for f in frames if f.get("cars")}
            out = subprocess.run([str(VERIFY), "--wheel-trace", str(capture), "0.5", "100"], capture_output=True, text=True).stdout
            rows = []
            for line in out.splitlines():
                m = LINE.match(line)
                if m:
                    touching = tuple(name for name, kind in WHEELS.findall(line) if kind == "n(")
                    rows.append((round(float(m.group(1)), 3), touching, m.group(7), (float(m.group(8)), float(m.group(9)), float(m.group(10)))))
            for i in range(1, len(rows) - window):
                t, touching, flags, _ = rows[i]
                if not touching or rows[i - 1][1] or "J" in flags or "J" in rows[i - 1][2]:
                    continue
                car = by_time.get(t)
                if car is None:
                    continue
                total = [0.0, 0.0, 0.0]
                for _, _, _, err in rows[i : i + window]:
                    local = rotate_inverse(car["rotation"], err)
                    for k in range(3):
                        total[k] += local[k]
                events.append((capture.name[:-10], t, touching, car["velocity"]["z"], total))
    if not events:
        print("no touchdowns")
        return 0
    print(f"{len(events)} touchdowns, error summed over {window} ticks, car frame (fwd, left, up) uu/s")
    for k, name in enumerate(("forward", "left", "up")):
        values = [e[4][k] for e in events]
        print(f"  {name:<8} mean {statistics.fmean(values):7.2f}  median {statistics.median(values):7.2f}  rms {math.sqrt(statistics.fmean(v * v for v in values)):7.2f}")
    by_set: dict = {}
    for e in events:
        by_set.setdefault(len(e[2]), []).append(e)
    for n, group in sorted(by_set.items()):
        ups = [e[4][2] for e in group]
        fwds = [e[4][0] for e in group]
        print(f"  {n} wheels first: {len(group):>4} landings, up mean {statistics.fmean(ups):6.2f} median {statistics.median(ups):6.2f}, fwd mean {statistics.fmean(fwds):6.2f} median {statistics.median(fwds):6.2f}")
    print("worst by |error|:")
    for e in sorted(events, key=lambda e: -math.sqrt(sum(c * c for c in e[4])))[:6]:
        print(f"  {e[0]} t={e[1]:.3f} wheels {'+'.join(e[2])} vz {e[3]:7.1f}  err ({e[4][0]:6.1f}, {e[4][1]:6.1f}, {e[4][2]:6.1f})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
