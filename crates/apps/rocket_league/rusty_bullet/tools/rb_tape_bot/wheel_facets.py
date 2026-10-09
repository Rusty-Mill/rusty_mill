"""Do one-step velocity errors line up with wheel facet changes? (PARITY-PLAN A.1)

    python -I tools/rb_tape_bot/wheel_facets.py replays/batch_<stamp>/wallg_*_run1.jsonl

Runs `rb-verify --wheel-trace` over each capture and compares the one-step velocity
error on ticks where any wheel's hit normal changed against the ticks where none did.
A facet crossing that explains the wall error shows as a much larger mean error on the
FACET ticks, and an error that has a direction (here its mean vector in world axes).
"""
import re
import statistics
import subprocess
import sys

ROOT = __file__.replace("\\", "/").rsplit("/tools/", 1)[0]
VERIFY = ROOT + "/target/release/rb-verify.exe"
LINE = re.compile(r"dv err \(\s*([-\d.]+),\s*([-\d.]+),\s*([-\d.]+)\) \|\s*([\d.]+)\|")


def trace(path: str) -> list[tuple[float, tuple[float, float, float], bool]]:
    out = subprocess.run([VERIFY, "--wheel-trace", path, "0", "100"], capture_output=True, text=True).stdout
    rows = []
    for line in out.splitlines():
        m = LINE.search(line)
        if m:
            rows.append((float(m.group(4)), (float(m.group(1)), float(m.group(2)), float(m.group(3))), line.rstrip().endswith("FACET")))
    return rows


def main() -> int:
    if len(sys.argv) < 2:
        print(__doc__)
        return 2
    print(f"{'capture':<34}{'ticks':>6}{'facet':>6}{'|dv| facet':>11}{'|dv| other':>11}{'ratio':>7}")
    for path in sys.argv[1:]:
        rows = trace(path)
        # Only ticks with the car on a surface: the airborne start is not what is measured.
        facet = [r[0] for r in rows if r[2]]
        other = [r[0] for r in rows if not r[2]]
        if not facet or not other:
            continue
        mf, mo = statistics.mean(facet), statistics.mean(other)
        name = path.replace("\\", "/").rsplit("/", 1)[-1]
        print(f"{name:<34}{len(rows):>6}{len(facet):>6}{mf:>11.2f}{mo:>11.2f}{mf / mo if mo else 0:>7.1f}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
