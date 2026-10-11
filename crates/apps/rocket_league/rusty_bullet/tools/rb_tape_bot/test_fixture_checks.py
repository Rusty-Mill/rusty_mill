"""Tests for fixture_checks.py: input handling, the boost activation reader and the friction estimator.

    python -I -m unittest discover -s tools/rb_tape_bot -p "test_fixture_checks.py"
"""
import importlib.util
import json
import math
import pathlib
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location("fixture_checks", pathlib.Path(__file__).resolve().parent / "fixture_checks.py")
fc = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(fc)
DT = fc.DT


def write(lines):
    path = pathlib.Path(tempfile.mkdtemp()) / "cap.jsonl"
    path.write_text("\n".join(lines) + "\n")
    return path


def burn(presses, ticks, start=100.0, skip=()):
    """A synthetic series: boost pressed on the frames in `presses`, the tank draining on the
    `ticks` frames after the first press (min-burn style), samples in `skip` missing."""
    series, amount = [], start
    for k in range(40):
        if k in ticks:
            amount -= 100 / 3 / 120
        if k not in skip:
            series.append((k * DT, k in presses, amount))
    return series


class Load(unittest.TestCase):
    def test_reads_valid_records_and_blank_lines(self):
        path = write(['{"a": 1}', "", '{"a": 2}'])
        self.assertEqual(fc.load(path), [{"a": 1}, {"a": 2}])

    def test_skips_a_truncated_final_record(self):
        path = write(['{"a": 1}', '{"a": 2}', '{"a": '])
        self.assertEqual(fc.load(path), [{"a": 1}, {"a": 2}])

    def test_rejects_a_malformed_middle_record_with_file_and_line(self):
        path = write(['{"a": 1}', '{"a": ', '{"a": 3}'])
        with self.assertRaises(ValueError) as caught:
            fc.load(path)
        self.assertIn("cap.jsonl:2", str(caught.exception))


class Activations(unittest.TestCase):
    def test_a_short_tap_burns_twelve_ticks(self):
        out = fc.activations(burn(presses={5, 6, 7}, ticks=range(7, 19)))
        self.assertEqual(len(out), 1)
        self.assertEqual((out[0]["held"], out[0]["drops"], out[0]["span_ticks"], out[0]["gaps"]), (3, 12, 12, 0))

    def test_an_empty_tank_press_has_no_drain_and_does_not_crash(self):
        out = fc.activations(burn(presses={5, 6}, ticks=(), start=0.0))
        self.assertEqual(out[0]["drops"], 0)
        self.assertEqual(out[0]["span_ticks"], 0)

    def test_a_missing_sample_is_a_gap_not_a_shorter_burn(self):
        out = fc.activations(burn(presses={5, 6}, ticks=range(7, 19), skip={12}))
        self.assertEqual(out[0]["span_ticks"], 12)
        self.assertEqual(out[0]["gaps"], 1)

    def test_a_second_press_is_not_counted_in_the_first_burn(self):
        series = burn(presses={5, 6, 14, 15}, ticks=list(range(7, 19)) + list(range(16, 28)))
        first, second = fc.activations(series)
        self.assertEqual(first["span_ticks"], 7)  # stops where the second activation starts
        self.assertEqual(second["held"], 2)

    def test_long_holds_are_left_out(self):
        self.assertEqual(fc.activations(burn(presses=set(range(5, 30)), ticks=range(6, 30))), [])


class Plateaus(unittest.TestCase):
    def test_a_cap_reached_late_from_a_high_value_is_dynamic(self):
        late = [10.0] * 25 + [29.0] + [30.0] * 3
        self.assertEqual(fc.plateaus(late, 30.0), [(26, 3, 29.0, True)])

    def test_a_seeded_start_is_not_dynamic(self):
        self.assertEqual(fc.plateaus([30.0] * 3 + [10.0] * 25, 30.0), [(0, 3, None, False)])

    def test_a_one_step_jump_from_rest_is_not_dynamic(self):
        self.assertFalse(fc.plateaus([0.0] * 25 + [30.0] * 3, 30.0)[0][3])

    def test_values_just_under_the_cap_are_not_a_plateau(self):
        self.assertEqual(fc.plateaus([29.0] * 30, 30.0), [])


class Friction(unittest.TestCase):
    def test_recovers_the_friction_under_exponential_drag(self):
        friction, v, rows = 228.0, 1500.0, []
        for k in range(300):
            rows.append((k * DT, v, 0.0, 93.2))
            v = v * (1 - fc.BALL_DRAG) ** DT - friction * DT
        est = fc.friction_samples(rows)
        self.assertGreater(len(est), 100)
        self.assertAlmostEqual(sum(est) / len(est), friction, delta=0.5)

    def test_ignores_slow_and_airborne_samples(self):
        rows = [(k * DT, 500.0, 0.0, 93.2) for k in range(5)] + [(k * DT, 1500.0, 0.0, 150.0) for k in range(5, 10)]
        self.assertEqual(fc.friction_samples(rows), [])


if __name__ == "__main__":
    unittest.main()
