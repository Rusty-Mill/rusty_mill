"""Does the k-step error grow with the wheel facets crossed in its window? (PARITY-PLAN A)

    python -I tools/rb_tape_bot/wheel_kstep.py [k] replays/batch_<stamp>/wallg_*_run1.jsonl

Runs `rb-verify --wheel-kstep` (default k = 30, a quarter second) and buckets the ticks
where all four wheels touched at the window's start by the number of wheel facet changes
inside the window. A steady force bias is absorbed by the suspension within a window, so
a mean error that rises with the changes is a force difference at the crossings; a flat
one says the facets are not the cause.
"""
import re
import statistics
import subprocess
import sys

ROOT = __file__.replace("\\", "/").rsplit("/tools/", 1)[0]
VERIFY = ROOT + "/target/release/rb-verify.exe"
LINE = re.compile(
    r"window front\s+(\d+) rear\s+(\d+) touching (\d) \| pos err \(\s*([-\d.]+),\s*([-\d.]+),\s*([-\d.]+)\) \|\s*([\d.]+)\| "
    r"vel err \(\s*([-\d.]+),\s*([-\d.]+),\s*([-\d.]+)\) \|\s*([\d.]+)\|"
)
BUCKETS = [(0, 0), (1, 2), (3, 5), (6, 99)]


def rows_of(path: str, k: int) -> list[tuple[int, int, float, float]]:
    out = subprocess.run([VERIFY, "--wheel-kstep", path, str(k)], capture_output=True, text=True).stdout
    rows = []
    for line in out.splitlines():
        m = LINE.search(line)
        if m and m.group(3) == "4":
            rows.append((int(m.group(1)), int(m.group(2)), float(m.group(7)), float(m.group(11))))
    return rows


def main() -> int:
    args = sys.argv[1:]
    k = 30
    if args and args[0].isdigit():
        k, args = int(args[0]), args[1:]
    if not args:
        print(__doc__)
        return 2
    pooled: dict[tuple[int, int], list[tuple[float, float]]] = {b: [] for b in BUCKETS}
    print(f"k = {k}; mean k-step position error (uu) / velocity error (uu/s), four wheels down at the window start")
    print(f"{'capture':<30}" + "".join(f"{f'{lo}-{hi} changes':>20}" for lo, hi in BUCKETS))
    for path in args:
        rows = rows_of(path, k)
        cells = []
        for lo, hi in BUCKETS:
            sel = [(r[2], r[3]) for r in rows if lo <= r[0] + r[1] <= hi]
            pooled[(lo, hi)].extend(sel)
            cells.append(f"{statistics.mean(p for p, _ in sel):6.2f}/{statistics.mean(v for _, v in sel):6.1f} n{len(sel):<4}" if sel else "-")
        name = path.replace("\\", "/").rsplit("/", 1)[-1]
        print(f"{name:<30}" + "".join(f"{c:>20}" for c in cells))
    print(f"{'ALL':<30}" + "".join(
        f"{(f'{statistics.mean(p for p, _ in v):6.2f}/{statistics.mean(x for _, x in v):6.1f} n{len(v):<4}' if v else '-'):>20}"
        for v in pooled.values()
    ))
    return 0


if __name__ == "__main__":
    sys.exit(main())
