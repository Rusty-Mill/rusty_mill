"""Seeded random ball launches, for a holistic check of the ball and the arena.

    python -I tools/rb_tape_bot/gen_ball_fuzz.py [count [first_seed]]

Writes tools/rb_tape_bot/experiments/ballfuzz_<seed>.json: the ball set flying
from a random place with a random velocity and spin, the car parked on the far
side doing nothing, 6 s of bouncing off the floor, walls, ceiling, corners,
goals and posts. Run with `run_batch.ps1 -Scenarios ballfuzz_1,...` and score
with `rb-verify --scenario ... --against ... --recorded-inputs` (the ball error
column), or `ball_fuzz_report.py <batch>`.
"""
import json
import pathlib
import random
import sys

OUT = pathlib.Path(__file__).resolve().parent / "experiments"
TICKS = 720  # 6 s


def main() -> None:
    count = int(sys.argv[1]) if len(sys.argv) > 1 else 24
    first = int(sys.argv[2]) if len(sys.argv) > 2 else 201
    names = []
    for seed in range(first, first + count):
        rng = random.Random(seed)
        x, y, z = rng.randint(-3000, 3000), rng.randint(-4000, 4000), rng.randint(150, 1200)
        speed = rng.uniform(500, 2500)
        direction = [rng.gauss(0, 1) for _ in range(3)]
        norm = sum(c * c for c in direction) ** 0.5
        velocity = [round(speed * c / norm, 1) for c in direction]
        spin = [round(rng.uniform(-5, 5), 2) for _ in range(3)]
        car_x = -3000 if x > 0 else 3000
        car_y = -4000 if y > 0 else 4000
        scenario = {
            "name": f"ballfuzz_{seed}: a seeded random ball launch, 6 s",
            "settle_ticks": 60,
            "car": {"location": [car_x, car_y, 17], "rotation": [-0.0096, 0, 0], "velocity": [0, 0, 0],
                    "angular_velocity": [0, 0, 0], "boost": 0},
            "ball": {"location": [x, y, z], "velocity": velocity, "angular_velocity": spin},
            "steps": [{"ticks": TICKS, "throttle": 0}],
        }
        (OUT / f"ballfuzz_{seed}.json").write_text(json.dumps(scenario, indent=2) + "\n", encoding="utf-8", newline="\n")
        names.append(f"ballfuzz_{seed}")
    print(",".join(names))


if __name__ == "__main__":
    main()
