"""Cross-validation of rusty_erosion against SlopCodeBench (card #705 Phase A, D201).

    scb-check ... | python3 summarize.py scb   # one scb-check JSON report -> 4 TSV fields
    python3 summarize.py report ROWS.tsv       # the pre-registered comparison, as Markdown

Pre-registered rules (705-slopcodebench-spike-design.md §3 A.4):
- concentration vs SCB erosion agrees when Spearman rho >= 0.6 on levels AND the per-step
  direction agrees on >= 75% of checkpoint steps where at least one of the two moved;
- duplication vs SCB's clone-line share agrees when rho >= 0.5.
Duplication vs verbosity is reported without a verdict.
"""
import json
import math
import sys
from typing import NamedTuple

RHO_CONC = 0.6
SIGN_CONC = 0.75
RHO_DUP = 0.5
WORST = 5


class Row(NamedTuple):
    problem: str
    checkpoint: int
    functions: int
    total_cc: int
    conc: float
    dup: float
    erosion: float
    verbosity: float
    clone: float


def share(num: str, den: str) -> float:
    return int(num) / int(den) if int(den) else 0.0


def parse(line: str) -> Row:
    f = line.rstrip("\n").split("\t")
    if len(f) != 14:
        raise ValueError(f"expected 14 fields, got {len(f)}: {line!r}")
    return Row(
        problem=f[0],
        checkpoint=int(f[1]),
        functions=int(f[4]),
        total_cc=int(f[3]),
        conc=share(f[5], f[6]),
        dup=share(f[7], f[8]),
        erosion=float(f[10]),
        verbosity=float(f[11]),
        clone=share(f[12], f[13]),
    )


def scb_fields(report: dict) -> str:
    """The four scb-check report fields the rows keep; a missing one is an error."""
    keys = ("erosion", "verbosity", "clone_loc", "total_loc")
    missing = [k for k in keys if not isinstance(report.get(k), (int, float))]
    if missing:
        raise ValueError(f"scb-check report lacks {missing}")
    return "\t".join(str(report[k]) for k in keys)


def ranks(xs: list) -> list:
    """1-based ranks, ties sharing their average rank."""
    order = sorted(range(len(xs)), key=lambda i: xs[i])
    out = [0.0] * len(xs)
    i = 0
    while i < len(order):
        j = i
        while j + 1 < len(order) and xs[order[j + 1]] == xs[order[i]]:
            j += 1
        for k in range(i, j + 1):
            out[order[k]] = (i + j) / 2 + 1
        i = j + 1
    return out


def spearman(xs: list, ys: list) -> float:
    """Spearman's rho; NaN when either side is constant or there are under 3 points."""
    if len(xs) != len(ys):
        raise ValueError("length mismatch")
    if len(xs) < 3:
        return math.nan
    rx, ry = ranks(xs), ranks(ys)
    mx, my = sum(rx) / len(rx), sum(ry) / len(ry)
    cov = sum((a - mx) * (b - my) for a, b in zip(rx, ry))
    vx = sum((a - mx) ** 2 for a in rx)
    vy = sum((b - my) ** 2 for b in ry)
    return cov / math.sqrt(vx * vy) if vx and vy else math.nan


def steps(rows: list) -> list:
    """Consecutive (before, after) pairs within each problem, by checkpoint."""
    by = {}
    for r in rows:
        by.setdefault(r.problem, []).append(r)
    out = []
    for problem in sorted(by):
        seq = sorted(by[problem], key=lambda r: r.checkpoint)
        out.extend(zip(seq, seq[1:]))
    return out


def sign(x: float) -> int:
    return (x > 0) - (x < 0)


def sign_agreement(pairs: list, ours: str, theirs: str) -> tuple:
    """(agreeing, counted) over steps where at least one metric moved."""
    agree = counted = 0
    for a, b in pairs:
        s1 = sign(getattr(b, ours) - getattr(a, ours))
        s2 = sign(getattr(b, theirs) - getattr(a, theirs))
        if s1 == 0 and s2 == 0:
            continue
        counted += 1
        agree += s1 == s2
    return agree, counted


def verdict(ok: bool) -> str:
    return "agree" if ok else "disagree"


def report(rows: list) -> str:
    pairs = steps(rows)
    col = lambda name: [getattr(r, name) for r in rows]  # noqa: E731
    rho_c = spearman(col("conc"), col("erosion"))
    rho_d = spearman(col("dup"), col("clone"))
    rho_v = spearman(col("dup"), col("verbosity"))
    agree_c, n_c = sign_agreement(pairs, "conc", "erosion")
    agree_d, n_d = sign_agreement(pairs, "dup", "clone")
    frac_c = agree_c / n_c if n_c else math.nan
    conc_ok = rho_c >= RHO_CONC and frac_c >= SIGN_CONC
    lines = [
        f"Snapshots: {len(rows)} in {len({r.problem for r in rows})} problems; steps: {len(pairs)}.",
        "",
        "| Pair | Spearman rho | Direction agrees | Rule | Verdict |",
        "|---|---|---|---|---|",
        f"| concentration vs SCB erosion | {rho_c:.3f} | {agree_c}/{n_c} ({frac_c:.0%}) "
        f"| rho >= {RHO_CONC} and >= {SIGN_CONC:.0%} | {verdict(conc_ok)} |",
        f"| duplication vs SCB clone share | {rho_d:.3f} | {agree_d}/{n_d} "
        f"| rho >= {RHO_DUP} | {verdict(rho_d >= RHO_DUP)} |",
        f"| duplication vs SCB verbosity | {rho_v:.3f} | | none | reported only |",
        "",
        "Worst concentration/erosion rank disagreements (gaming check: functions, total CC):",
        "",
        "| Problem | Checkpoint | concentration | SCB erosion | functions | total CC |",
        "|---|---|---|---|---|---|",
    ]
    rc, re_ = ranks(col("conc")), ranks(col("erosion"))
    worst = sorted(range(len(rows)), key=lambda i: -abs(rc[i] - re_[i]))[:WORST]
    for i in worst:
        r = rows[i]
        lines.append(
            f"| {r.problem} | {r.checkpoint} | {r.conc:.3f} | {r.erosion:.3f} "
            f"| {r.functions} | {r.total_cc} |"
        )
    return "\n".join(lines)


def main(argv: list) -> int:
    if argv[1:2] == ["scb"]:
        print(scb_fields(json.load(sys.stdin)))
        return 0
    if argv[1:2] == ["report"] and len(argv) == 3:
        with open(argv[2], encoding="utf-8") as fh:
            next(fh)
            print(report([parse(line) for line in fh if line.strip()]))
        return 0
    print(__doc__, file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv))
