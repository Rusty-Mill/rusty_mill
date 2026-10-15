"""Tapes that probe the wheel brake on flat ground from a settled car (no state-set transients).

    python -I tools/rb_tape_bot/gen_brake_probe.py

The car sits at rest on the floor for 60 ticks (suspension settled), then gets its speed from a short, known throttle
(one tick of full throttle adds about 13.3 uu/s) and coasts with no input. Writes experiments/brake_*.json, prints names:
- ``brake_ring2_{f,r}{6,13,20}``: one tick of throttle (0.5; 1.0; 1.0 then 0.5) forward (f) or backward (r), then 45 idle
  ticks. Below the brake's clamp each tick multiplies the speed by the same negative factor (the car rings).
- ``brake_sat2_{200,400}_rev``: throttle 1 for 15 (30) ticks to reach about 200 (400) uu/s, then throttle -1 for 3 ticks
  (full brake against the motion), then idle: the speed loss per tick while the brake is clamped is its force.
- ``brake_sat2_200_coast``: the same run-up then idle (the coasting brake alone).
Run with `run_batch.ps1 -Repeat 3 -Scenarios <names>`.
"""
import json
import pathlib

EXP = pathlib.Path(__file__).resolve().parent / "experiments"
HEADING_YAW = 1.5708  # faces +y


def tape(name: str, doc: str, steps) -> dict:
    return {
        "name": f"{name}: {doc}",
        "settle_ticks": 60,
        "car": {
            "location": [0, -3000, 17],
            "rotation": [0, HEADING_YAW, 0],
            "velocity": [0, 0, 0],
            "angular_velocity": [0, 0, 0],
            "boost": 0,
        },
        "ball": {"location": [3500, 3500, 93.15], "velocity": [0, 0, 0]},
        "steps": steps,
    }


def all_tapes() -> dict[str, dict]:
    out: dict[str, dict] = {}
    kicks = {"6": [(0.5, 1)], "13": [(1.0, 1)], "20": [(1.0, 1), (0.5, 1)]}
    for sign, tag in ((1, "f"), (-1, "r")):
        for label, kick in kicks.items():
            name = f"brake_ring2_{tag}{label}"
            steps = [{"ticks": n, "throttle": sign * t} for t, n in kick] + [{"ticks": 45}]
            out[name] = tape(name, f"from rest, throttle {[sign * t for t, _ in kick]} for one tick each, then no input", steps)
    for speed, ticks in ((200, 15), (400, 30)):
        name = f"brake_sat2_{speed}_rev"
        out[name] = tape(name, f"throttle 1 for {ticks} ticks, throttle -1 for 3 ticks, then none",
                         [{"ticks": ticks, "throttle": 1}, {"ticks": 3, "throttle": -1}, {"ticks": 45}])
    out["brake_sat2_200_coast"] = tape("brake_sat2_200_coast", "throttle 1 for 15 ticks, then no input",
                                       [{"ticks": 15, "throttle": 1}, {"ticks": 48}])
    return out


def main() -> None:
    names = []
    for name, doc in all_tapes().items():
        (EXP / f"{name}.json").write_text(json.dumps(doc, indent=2) + chr(10), encoding="utf-8", newline=chr(10))
        names.append(name)
    print(",".join(names))


if __name__ == "__main__":
    main()
