"""Summarize measure.sh output for the erosion calibration report (card #704 slice 5, D110).

    python3 summarize.py rows OUT > ROWS.tsv   # one row per PR, the fields the report uses
    python3 summarize.py report ROWS.tsv       # the report's tables

`rows` keeps a run auditable without its 1 MB of raw erosion lines: number, branch, merge
commit, the two share deltas, the function-count and total-CC deltas, unmeasured files by
extension (D45) and whether erosion errored. `report` prints, for the strict set (branches not
under claude/, codex/ or import/; OPEN_QUESTIONS Q36) and for every non-bot PR except import/
subtree imports: p50/p90/p95/p99 of each delta, how many PRs a limit pair would fail, unmeasured
files by extension, and the gaming signal (concentration fell while function count or total CC
rose).
"""
import collections
import glob
import json
import math
import sys
from typing import NamedTuple

AGENT = ("claude/", "codex/", "import/")
LIMITS = (0, 1, 2, 5, 10, 25)
HEADER = "number\tbranch\tsha\tconcentration_bp\tduplication_bp\tfunctions\ttotal_cc\tunmeasured\terror"


class Row(NamedTuple):
    number: int
    branch: str
    sha: str
    concentration: int
    duplication: int
    functions: int
    total_cc: int
    unmeasured: dict[str, int]
    error: bool


def from_line(number: str, branch: str, sha: str, line: dict) -> Row:
    if "error" in line:
        return Row(int(number), branch, sha, 0, 0, 0, 0, {}, True)
    exts = collections.Counter(u.rsplit(".", 1)[-1] for u in line["unmeasured"])
    return Row(
        int(number), branch, sha,
        line["concentration_bp"]["delta"], line["duplication_bp"]["delta"],
        line["functions"], line["total_cc"], dict(sorted(exts.items())), False,
    )


def rows_from_out(out: str) -> list[Row]:
    rows = []
    for path in glob.glob(f"{out}/*.tsv"):
        number, branch, sha, line = open(path).read().rstrip("\n").split("\t", 3)
        rows.append(from_line(number, branch, sha, json.loads(line)))
    return sorted(rows)


def to_tsv(r: Row) -> str:
    exts = ",".join(f"{e}:{n}" for e, n in r.unmeasured.items())
    cells = (r.number, r.branch, r.sha, r.concentration, r.duplication, r.functions, r.total_cc, exts, int(r.error))
    return "\t".join(map(str, cells))


def from_tsv(text: str) -> Row:
    n, branch, sha, c, d, f, cc, exts, err = text.rstrip("\n").split("\t")
    unmeasured = {e: int(k) for e, k in (x.split(":") for x in exts.split(",") if x)}
    return Row(int(n), branch, sha, int(c), int(d), int(f), int(cc), unmeasured, err == "1")


def rank(xs: list[int], p: int) -> int:
    """Nearest-rank percentile."""
    xs = sorted(xs)
    return xs[max(0, math.ceil(p / 100 * len(xs)) - 1)]


def report(name: str, rows: list[Row]) -> None:
    ok = [r for r in rows if not r.error]
    print(f"\n### {name}: {len(rows)} PRs, {len(rows) - len(ok)} could not be measured\n")
    print("| Metric | p50 | p90 | p95 | p99 | max | PRs that raise it |")
    print("| --- | --- | --- | --- | --- | --- | --- |")
    for key in ("concentration", "duplication"):
        xs = [getattr(r, key) for r in ok]
        cells = " | ".join(str(rank(xs, q)) for q in (50, 90, 95, 99))
        print(f"| {key}_bp | {cells} | {max(xs)} | {sum(x > 0 for x in xs)} |")
    print("\n| Limit pair (bp) | PRs it would fail |")
    print("| --- | --- |")
    for lim in LIMITS:
        n = sum(r.concentration > lim or r.duplication > lim for r in ok)
        print(f"| {lim} / {lim} | {n} ({100 * n / len(ok):.1f}%) |")
    exts = sum((collections.Counter(r.unmeasured) for r in ok), collections.Counter())
    touched = sum(bool(r.unmeasured) for r in ok)
    print(f"\nUnmeasured: {touched} PRs changed {sum(exts.values())} files erosion can't measure ({', '.join(f'{e} {n}' for e, n in exts.most_common(6))}).")
    gaming = [r for r in ok if r.concentration < 0 and (r.functions > 0 or r.total_cc > 0)]
    print(f"Gaming signal: {len(gaming)} PRs lowered concentration while adding functions or CC.")


def main() -> None:
    match sys.argv[1:]:
        case ["rows", out]:
            print(HEADER)
            for r in rows_from_out(out):
                print(to_tsv(r))
        case ["report", path]:
            lines = open(path).read().splitlines()
            if lines[:1] != [HEADER]:
                sys.exit(f"{path}: not a rows file")
            rows = [from_tsv(x) for x in lines[1:]]
            report("Strict set", [r for r in rows if not r.branch.startswith(AGENT)])
            report("All non-bot PRs except import/", [r for r in rows if not r.branch.startswith("import/")])
        case _:
            sys.exit(__doc__)


if __name__ == "__main__":
    main()
