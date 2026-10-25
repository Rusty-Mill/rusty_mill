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



class Freeze(unittest.TestCase):
    def rows(self, moving, held, start=(40.0, 0.44)):
        """The state set at `start`, `moving` frames slowing by 0.01 uu/s a tick, then `held` at zero."""
        rows = [(0.0, *start)]
        for k in range(1, moving + 1):
            rows.append((k * DT, start[0] - 0.01 * k, start[1] - 0.0001 * k))
        return rows + [((moving + 1 + k) * DT, 0.0, 0.0) for k in range(held)]

    def test_finds_the_freeze_and_the_most_speed_and_spin_while_timed(self):
        k, secs, speed, spin = fc.freeze(self.rows(240, 5))
        self.assertEqual(k, 241)
        self.assertAlmostEqual(secs, 241 * DT)
        self.assertAlmostEqual(speed, 39.99)
        self.assertAlmostEqual(spin, 0.4399)

    def test_a_ball_that_never_stops_has_no_freeze(self):
        self.assertIsNone(fc.freeze(self.rows(240, 0)))

    def test_a_ball_set_at_rest_is_not_a_freeze(self):
        self.assertIsNone(fc.freeze([(k * DT, 0.0, 0.0) for k in range(10)]))


class AwakeBound(unittest.TestCase):
    def rows(self, ticks, speed=(80.0, -0.02), spin=(0.88, -0.0002)):
        """The state set, then `ticks` stepped frames with speed and spin falling linearly."""
        return [(k * DT, speed[0] + speed[1] * k, spin[0] + spin[1] * k) for k in range(ticks + 1)]

    def test_bounds_by_the_slowest_full_window_not_the_slowest_frame(self):
        # 600 ticks: the last frame is at 68.0 uu/s, but a window of 240 ticks ending there
        # peaks 240 ticks earlier, at 72.8 uu/s and 0.808 rad/s.
        speed, spin = fc.awake_bound(self.rows(600), 240 * DT)
        self.assertAlmostEqual(speed, 80.0 - 0.02 * 360)
        self.assertAlmostEqual(spin, 0.88 - 0.0002 * 360)

    def test_reports_the_window_spin_beside_its_speed(self):
        # A spin that rises at the end: the slowest-speed window carries it, so the bound
        # stays a disjunction (linear or angular), never a linear bound alone.
        rows = self.rows(600, spin=(0.5, 0.001))
        _, spin = fc.awake_bound(rows, 240 * DT)
        self.assertAlmostEqual(spin, 0.5 + 0.001 * 600)

    def test_a_recording_shorter_than_the_dwell_bounds_nothing(self):
        self.assertIsNone(fc.awake_bound(self.rows(100), 240 * DT))

if __name__ == "__main__":
    unittest.main()
