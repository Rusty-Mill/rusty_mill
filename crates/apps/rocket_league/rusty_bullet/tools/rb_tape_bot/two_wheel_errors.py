"""One-step velocity error of ticks with exactly two wheels touching, in the car's frame.

    python -I tools/rb_tape_bot/two_wheel_errors.py replays/batch_<stamp>... [min_err]

For every `*_run1.jsonl` it runs `rb-verify --wheel-trace` and keeps the ticks where exactly
two wheels touch and no jump is pressed. Each tick's `dv err` (recorded minus predicted
velocity change) is turned into the car's frame (forward, left, up) with the recorded
rotation, and grouped by which wheels touch. Prints per group the tick count and the mean
error per tick on each axis, so a missing force with a fixed direction in the car frame
(a suspension term) stands out from one in the world frame (gravity, the sticky force).
"""
import collections
import json
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


def main() -> int:
    args = sys.argv[1:]
    min_err = 0.0
    if args and re.fullmatch(r"[\d.]+", args[-1]):
        min_err = float(args.pop())
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
                if not m or "J" in m.group(7):
                    continue
                touching = tuple(name for name, kind in WHEELS.findall(line) if kind == "n(")
                if len(touching) != int(__import__("os").environ.get("N_WHEELS", "2")):
                    continue
                q = rotation_at.get(round(float(m.group(1)), 3))
                if q is None:
                    continue
                err = (float(m.group(8)), float(m.group(9)), float(m.group(10)))
                if float(m.group(11)) < min_err:
                    continue
                local = rotate_inverse(q, err)
                groups[(touching, "thr" + ("+" if float(m.group(2)) > 0.3 else "-" if float(m.group(2)) < -0.3 else "0"))].append((local, err, capture.name[:-10], float(m.group(1))))
    if len(sys.argv) > 0 and __import__('os').environ.get('TWO_WHEEL_DETAIL'):
        want = tuple(__import__('os').environ['TWO_WHEEL_DETAIL'].split('+'))
        for (wheels, thr), rows in groups.items():
            if wheels == want:
                per = collections.defaultdict(list)
                for local, _, name, t in rows:
                    per[name].append((t, local))
                for name, items in sorted(per.items(), key=lambda kv: -len(kv[1]))[:8]:
                    print(thr, name, len(items), 't %.2f-%.2f' % (items[0][0], items[-1][0]), 'fwd err %.2f' % (sum(i[1][0] for i in items) / len(items)))
    print(f"{'wheels':<14}{'thr':<6}{'ticks':>6}  car frame (fwd, left, up) uu/s per tick      world (x, y, z)")
    for (wheels, thr), rows in sorted(groups.items(), key=lambda kv: -len(kv[1])):
        n = len(rows)
        mean = lambda i, which: sum(r[which][i] for r in rows) / n
        print(
            f"{'+'.join(wheels):<14}{thr:<6}{n:>6}  "
            f"({mean(0, 0):6.2f}, {mean(1, 0):6.2f}, {mean(2, 0):6.2f})   ({mean(0, 1):6.2f}, {mean(1, 1):6.2f}, {mean(2, 1):6.2f})"
        )
    flat = [r for rows in groups.values() for r in rows]
    if flat:
        clipped = [min(abs(r[0][0]), 3.0) for r in flat]
        print(f"all groups: {len(flat)} ticks, mean |forward error| (clipped at 3) {sum(clipped) / len(clipped):.3f} uu/s per tick")
    return 0


if __name__ == "__main__":
    sys.exit(main())
