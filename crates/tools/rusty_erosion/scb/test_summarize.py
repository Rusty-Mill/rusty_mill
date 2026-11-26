"""Checks for summarize.py: ranks with ties, Spearman, step pairing, the sign rule and the
pre-registered verdicts."""
import math
import pathlib
import unittest

import summarize as s

DOCS = pathlib.Path(__file__).resolve().parents[4] / "docs/projects/let-em-loose"


def row(problem: str, cp: int, conc: float, erosion: float, dup: float = 0.0, clone: float = 0.0) -> str:
    cn, cd = int(conc * 1000), 1000
    dn, dd = int(dup * 1000), 1000
    cl, tl = int(clone * 1000), 1000
    return f"{problem}\t{cp}\t10\t5\t2\t{cn}\t{cd}\t{dn}\t{dd}\t0\t{erosion}\t0.4\t{cl}\t{tl}"


class Ranks(unittest.TestCase):
    def test_ties_share_average_rank(self):
        self.assertEqual(s.ranks([10, 20, 20, 5]), [2.0, 3.5, 3.5, 1.0])

    def test_spearman(self):
        self.assertAlmostEqual(s.spearman([1, 2, 3, 4], [10, 20, 30, 40]), 1.0)
        self.assertAlmostEqual(s.spearman([1, 2, 3, 4], [4, 3, 2, 1]), -1.0)
        self.assertTrue(math.isnan(s.spearman([1, 1, 1], [1, 2, 3])))
        self.assertTrue(math.isnan(s.spearman([1, 2], [1, 2])))


class Steps(unittest.TestCase):
    def test_pairs_stay_within_a_problem_in_checkpoint_order(self):
        rows = [s.parse(row("b", 2, 0.2, 0.2)), s.parse(row("a", 1, 0.1, 0.1)),
                s.parse(row("b", 1, 0.1, 0.1)), s.parse(row("a", 2, 0.3, 0.3))]
        got = [(a.problem, a.checkpoint, b.checkpoint) for a, b in s.steps(rows)]
        self.assertEqual(got, [("a", 1, 2), ("b", 1, 2)])

    def test_steps_where_neither_moves_are_not_counted(self):
        rows = [s.parse(row("a", 1, 0.1, 0.5)), s.parse(row("a", 2, 0.1, 0.5)),
                s.parse(row("a", 3, 0.2, 0.4))]
        self.assertEqual(s.sign_agreement(s.steps(rows), "conc", "erosion"), (0, 1))


class Verdicts(unittest.TestCase):
    def test_agreeing_data_passes_both_rules(self):
        rows = [s.parse(row("a", i, i / 10, i / 9, dup=i / 20, clone=i / 15)) for i in range(1, 6)]
        out = s.report(rows)
        self.assertIn("| concentration vs SCB erosion | 1.000 | 4/4 (100%)", out)
        self.assertEqual(out.count("| agree |"), 2)

    def test_opposed_data_disagrees(self):
        rows = [s.parse(row("a", i, i / 10, 1 - i / 10)) for i in range(1, 6)]
        self.assertIn("| -1.000 | 0/4 (0%)", s.report(rows))
        self.assertIn("| disagree |", s.report(rows))


class Scb(unittest.TestCase):
    def test_scb_fields(self):
        self.assertEqual(s.scb_fields({"erosion": 0.5, "verbosity": 0.25, "clone_loc": 3,
                                       "total_loc": 40, "x": 1}), "0.5\t0.25\t3\t40")
        with self.assertRaises(ValueError):
            s.scb_fields({"erosion": 0.5})

    def test_parse_rejects_short_rows(self):
        with self.assertRaises(ValueError):
            s.parse("a\t1\t2")


class Published(unittest.TestCase):
    def test_committed_rows_reproduce_the_phase_a_table(self):
        with open(DOCS / "705-slopcodebench-rows-2026-11-26.tsv", encoding="utf-8") as fh:
            next(fh)
            table = s.report([s.parse(line) for line in fh if line.strip()])
        published = (DOCS / "705-slopcodebench-phase-a.md").read_text(encoding="utf-8")
        self.assertIn(table, published)


if __name__ == "__main__":
    unittest.main()
