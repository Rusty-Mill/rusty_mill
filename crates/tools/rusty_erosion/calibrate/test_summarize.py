"""Checks for summarize.py: the row format, the percentile and limit arithmetic, and that the
committed 2026-11-05 rows still produce the tables published in erosion-calibration.md."""
import contextlib
import io
import pathlib
import unittest

import summarize as s

ROOT = pathlib.Path(__file__).resolve().parents[4]
DOCS = ROOT / "docs/projects/let-em-loose"


def line(c: int, d: int, functions: int = 0, cc: int = 0, unmeasured: tuple = ()) -> dict:
    return {
        "concentration_bp": {"base": 100, "candidate": 100 + c, "delta": c},
        "duplication_bp": {"base": 50, "candidate": 50 + d, "delta": d},
        "functions": functions,
        "total_cc": cc,
        "unmeasured": list(unmeasured),
    }


def printed(rows: list) -> str:
    out = io.StringIO()
    with contextlib.redirect_stdout(out):
        s.report("Set", rows)
    return out.getvalue()


class Rows(unittest.TestCase):
    def test_a_line_keeps_deltas_and_counts_unmeasured_by_extension(self):
        r = s.from_line("7", "feat/x", "abc", line(2, -1, 3, 4, ("a/b.ts", "c.ts", "d.py")))
        self.assertEqual(r, s.Row(7, "feat/x", "abc", 2, -1, 3, 4, {"py": 1, "ts": 2}, False))
        self.assertEqual(s.from_tsv(s.to_tsv(r)), r)

    def test_an_errored_line_is_flagged_and_round_trips(self):
        r = s.from_line("8", "b", "def", {"erosion": 1, "error": "bad"})
        self.assertTrue(r.error)
        self.assertEqual(s.from_tsv(s.to_tsv(r)), r)


class Report(unittest.TestCase):
    def test_nearest_rank_percentile(self):
        xs = list(range(1, 11))
        self.assertEqual([s.rank(xs, p) for p in (50, 90, 95, 99)], [5, 9, 10, 10])
        self.assertEqual(s.rank([0, 0, 0, 7], 90), 7)

    def test_a_pair_fails_a_pr_on_either_delta_above_the_limit(self):
        rows = [s.from_line(str(n), "b", "x", line(c, d)) for n, (c, d) in enumerate([(0, 0), (1, 0), (0, 2), (-3, 0)])]
        out = printed(rows)
        self.assertIn("| 0 / 0 | 2 (50.0%) |", out)
        self.assertIn("| 1 / 1 | 1 (25.0%) |", out)
        self.assertIn("| 2 / 2 | 0 (0.0%) |", out)

    def test_errors_are_counted_but_not_measured_and_gaming_needs_growth(self):
        rows = [
            s.from_line("1", "b", "x", line(-1, 0, functions=1)),
            s.from_line("2", "b", "x", line(-1, 0)),
            s.from_line("3", "b", "x", {"erosion": 1, "error": "bad"}),
        ]
        out = printed(rows)
        self.assertIn("3 PRs, 1 could not be measured", out)
        self.assertIn("Gaming signal: 1 PRs", out)


class Published(unittest.TestCase):
    def test_the_committed_rows_produce_the_published_tables(self):
        text = (DOCS / "erosion-calibration-2026-11-05.tsv").read_text().splitlines()
        self.assertEqual(text[0], s.HEADER)
        rows = [s.from_tsv(x) for x in text[1:]]
        self.assertEqual(len(rows), 1182)
        report = (DOCS / "erosion-calibration.md").read_text()
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            s.report("Strict", [r for r in rows if not r.branch.startswith(s.AGENT)])
            s.report("Wide", [r for r in rows if not r.branch.startswith("import/")])
        tables = [x for x in out.getvalue().splitlines() if x.startswith("| ") and "---" not in x]
        self.assertTrue(tables)
        for row in tables:
            self.assertIn(row, report)


if __name__ == "__main__":
    unittest.main()
