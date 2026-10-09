"""Seeded random drives that start on a wall, for wall driving, the ceiling and wall jumps.

    python -I tools/rb_tape_bot/gen_wall_fuzz.py [count [first_seed]]

Writes tools/rb_tape_bot/experiments/wallfuzz_<seed>.json: the car on the +x or
-x side wall (rolled a quarter turn, wheels on the wall) at a random height and
a random speed along it, then the random tape of `gen_fuzz.py` for 6 s: driving
up and along the wall, onto the ceiling, off the wall with a jump or a dodge.
"""
import json
import math
import pathlib
import random
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from gen_fuzz import random_tape  # noqa: E402

OUT = pathlib.Path(__file__).resolve().parent / "experiments"
TICKS = 720
WALL_X = 4096
WHEEL_GAP = 17  # origin height above the surface when the wheels touch it


def main() -> None:
    count = int(sys.argv[1]) if len(sys.argv) > 1 else 16
    first = int(sys.argv[2]) if len(sys.argv) > 2 else 401
    names = []
    for seed in range(first, first + count):
        rng = random.Random(seed)
        side = rng.choice([1, -1])
        z = rng.randint(500, 1500)
        y = rng.randint(-2500, 2500)
        yaw = rng.choice([math.pi / 2, -math.pi / 2])
        speed = rng.randint(0, 900)
        # Rolled so the car's up axis points away from the wall (toward the arena).
        roll = side * math.pi / 2 * (1 if yaw > 0 else -1)
        velocity = [0, round(speed * math.sin(yaw)), 0]
        steps, total = [], 0
        for step in random_tape(rng):
            if total >= TICKS:
                break
            steps.append(step)
            total += step["ticks"]
        scenario = {
            "name": f"wallfuzz_{seed}: a random tape from the {'+' if side > 0 else '-'}x wall",
            "settle_ticks": 20,
            "car": {"location": [side * (WALL_X - WHEEL_GAP), y, z], "rotation": [0, round(yaw, 4), round(roll, 4)],
                    "velocity": velocity, "angular_velocity": [0, 0, 0], "boost": 100},
            "ball": {"location": [0, 4500, 93.15], "velocity": [0, 0, 0]},
            "steps": steps,
        }
        (OUT / f"wallfuzz_{seed}.json").write_text(json.dumps(scenario, indent=2) + "\n", encoding="utf-8", newline="\n")
        names.append(f"wallfuzz_{seed}")
    print(",".join(names))


if __name__ == "__main__":
    main()
