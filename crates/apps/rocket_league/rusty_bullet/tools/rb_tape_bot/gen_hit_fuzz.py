"""Seeded random drives that start at the ball, for car-ball and car-ball-wall combinations.

    python -I tools/rb_tape_bot/gen_hit_fuzz.py [count [first_seed]]

Writes tools/rb_tape_bot/experiments/hitfuzz_<seed>.json: the ball resting on the
floor and the car 600 to 900 uu away facing it, a run at it (throttle and boost)
and then the random tape of `gen_fuzz.py` for 6 s. Score with the ball error
column of `rb-verify --scenario ... --against ... --recorded-inputs`.
"""
import json
import math
import pathlib
import random
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from gen_fuzz import random_tape  # noqa: E402

OUT = pathlib.Path(__file__).resolve().parent / "experiments"


def main() -> None:
    count = int(sys.argv[1]) if len(sys.argv) > 1 else 24
    first = int(sys.argv[2]) if len(sys.argv) > 2 else 301
    names = []
    for seed in range(first, first + count):
        rng = random.Random(seed)
        bx, by = rng.randint(-1500, 1500), rng.randint(-2500, 2500)
        angle = rng.uniform(-math.pi, math.pi)
        distance = rng.uniform(600, 900)
        cx, cy = bx - distance * math.cos(angle), by - distance * math.sin(angle)
        cx, cy = max(-3500, min(3500, cx)), max(-4500, min(4500, cy))
        yaw = math.atan2(by - cy, bx - cx) + rng.uniform(-0.3, 0.3)
        run = [{"ticks": rng.randint(25, 45), "throttle": 1, "boost": True}]
        scenario = {
            "name": f"hitfuzz_{seed}: a run at the ball, then a seeded random tape",
            "settle_ticks": 60,
            "car": {"location": [round(cx), round(cy), 17], "rotation": [-0.0096, round(yaw, 4), 0],
                    "velocity": [0, 0, 0], "angular_velocity": [0, 0, 0], "boost": 100},
            "ball": {"location": [bx, by, 93.15], "velocity": [0, 0, 0]},
            "steps": run + random_tape(rng)[: 40],
        }
        # random_tape fills 720 ticks; keep the run plus the tape up to 720 ticks in all
        total = 0
        steps = []
        for step in scenario["steps"]:
            if total >= 720:
                break
            steps.append(step)
            total += step["ticks"]
        scenario["steps"] = steps
        (OUT / f"hitfuzz_{seed}.json").write_text(json.dumps(scenario, indent=2) + "\n", encoding="utf-8", newline="\n")
        names.append(f"hitfuzz_{seed}")
    print(",".join(names))


if __name__ == "__main__":
    main()
