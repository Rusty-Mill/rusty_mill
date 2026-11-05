"""Summarize measure.sh output for the erosion calibration report (card #704 slice 5, D110).

    python3 summarize.py OUT

Prints, for the strict set (branches not under claude/, codex/ or import/; OPEN_QUESTIONS Q36)
and for every non-bot PR except import/ subtree imports: p50/p90/p95/p99 of each delta, how
many PRs a limit pair would fail, unmeasured files by extension (D45), and the gaming signal
(concentration fell while function count or total CC rose).
"""
import collections
import glob
import json
import math
import sys
from typing import NamedTuple

AGENT = ("claude/", "codex/", "import/")
LIMITS = (0, 1, 2, 5, 10, 25)


class Pr(NamedTuple):
    number: int
    branch: str
    line: dict


def load(out: str) -> list[Pr]:
    prs = []
    for path in glob.glob(f"{out}/*.tsv"):
        number, branch, line = open(path).read().rstrip("\n").split("\t", 2)
        prs.append(Pr(int(number), branch, json.loads(line)))
    return sorted(prs)


def rank(xs: list[int], p: int) -> int:
    """Nearest-rank percentile."""
    xs = sorted(xs)
    return xs[max(0, math.ceil(p / 100 * len(xs)) - 1)]


def report(name: str, prs: list[Pr]) -> None:
    errors = [p for p in prs if "error" in p.line]
    ok = [p for p in prs if "error" not in p.line]
    print(f"\n### {name}: {len(prs)} PRs, {len(errors)} could not be measured\n")
    print("| Metric | p50 | p90 | p95 | p99 | max | PRs that raise it |")
    print("| --- | --- | --- | --- | --- | --- | --- |")
    for key in ("concentration_bp", "duplication_bp"):
        xs = [p.line[key]["delta"] for p in ok]
        cells = " | ".join(str(rank(xs, q)) for q in (50, 90, 95, 99))
        print(f"| {key} | {cells} | {max(xs)} | {sum(x > 0 for x in xs)} |")
    print("\n| Limit pair (bp) | PRs it would fail |")
    print("| --- | --- |")
    for lim in LIMITS:
        n = sum(p.line["concentration_bp"]["delta"] > lim or p.line["duplication_bp"]["delta"] > lim for p in ok)
        print(f"| {lim} / {lim} | {n} ({100 * n / len(ok):.1f}%) |")
    exts = collections.Counter(u.rsplit(".", 1)[-1] for p in ok for u in p.line["unmeasured"])
    touched = sum(bool(p.line["unmeasured"]) for p in ok)
    print(f"\nUnmeasured: {touched} PRs changed {sum(exts.values())} files erosion can't measure ({', '.join(f'{e} {n}' for e, n in exts.most_common(6))}).")
    gaming = [p for p in ok if p.line["concentration_bp"]["delta"] < 0 and (p.line["functions"] > 0 or p.line["total_cc"] > 0)]
    print(f"Gaming signal: {len(gaming)} PRs lowered concentration while adding functions or CC.")
    newest = max(ok).line
    print(f"Repo level after the newest PR: concentration {newest['concentration_bp']['candidate']} bp, duplication {newest['duplication_bp']['candidate']} bp.")


def main() -> None:
    prs = load(sys.argv[1])
    report("Strict set", [p for p in prs if not p.branch.startswith(AGENT)])
    report("All non-bot PRs except import/", [p for p in prs if not p.branch.startswith("import/")])


if __name__ == "__main__":
    main()
