"""Reads a single contact's impulse off a recording: where it acts, and along what.

    python -I tools/rb_tape_bot/impulse_fit.py <capture.jsonl> <frame> [car] [gravity]

`frame` is the capture row just after the contact (find it with the largest
velocity step, `--biggest`: `python -I impulse_fit.py <capture> --biggest`).
From the car's velocity and spin change over that one row, with the box's
inertia (Octane: 180, half extents 60.2535 x 43.3497 x 19.32955) and the
gravity of one tick added back, it prints:

- the linear impulse J (uu/s x mass) and its direction;
- the torque the spin change needs, and whether it is perpendicular to J (it
  is for one contact; the cosine says how much of a single contact it is);
- the line of action's closest approach to the centre of mass, in the car's
  frame. The same numbers from the port (`RB_STATES=1 rb-verify --scenario ...`)
  say whether the port's contact is at the same place and, from the direction,
  whether its normal and friction are the game's.

This is how the `probe_wall_land` miss (RB-RESEARCH-O017) was found to be a
different facet's normal, not a material: the game's direction matched the
upper facet's normal to 0.03 degrees at friction 0.3.
"""
import json
import math
import sys

HALF = (60.2535, 43.3497, 19.32955)
MASS = 180.0
GRAVITY = 650.0
TICK = 1.0 / 120.0


def rotation(q: dict) -> list:
    x, y, z, w = q["x"], q["y"], q["z"], q["w"]
    return [
        [1 - 2 * (y * y + z * z), 2 * (x * y - z * w), 2 * (x * z + y * w)],
        [2 * (x * y + z * w), 1 - 2 * (x * x + z * z), 2 * (y * z - x * w)],
        [2 * (x * z - y * w), 2 * (y * z + x * w), 1 - 2 * (x * x + y * y)],
    ]


def matvec(m: list, v: list) -> list:
    return [sum(m[i][j] * v[j] for j in range(3)) for i in range(3)]


def transpose(m: list) -> list:
    return [[m[j][i] for j in range(3)] for i in range(3)]


def dot(a: list, b: list) -> float:
    return sum(x * y for x, y in zip(a, b))


def cross(a: list, b: list) -> list:
    return [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]


def vec(c: dict, key: str) -> list:
    v = c[key]
    return [v["x"], v["y"], v["z"]] if isinstance(v, dict) else list(v)


def load(path: str) -> list:
    with open(path, encoding="utf-8") as f:
        return [json.loads(line) for line in f]


def biggest(rows: list, car: int) -> None:
    steps = []
    for i in range(1, len(rows)):
        a, b = vec(rows[i - 1]["cars"][car], "velocity"), vec(rows[i]["cars"][car], "velocity")
        steps.append((math.dist(a, b), i))
    for size, i in sorted(steps, reverse=True)[:8]:
        print(f"frame {i}: velocity step {size:.0f} uu/s")


def main() -> None:
    capture = sys.argv[1]
    rows = load(capture)
    car = int(sys.argv[3]) if len(sys.argv) > 3 and sys.argv[3].isdigit() else 0
    if sys.argv[2] == "--biggest":
        biggest(rows, car)
        return
    frame = int(sys.argv[2])
    gravity = float(sys.argv[4]) if len(sys.argv) > 4 else GRAVITY
    before, after = rows[frame - 1]["cars"][car], rows[frame]["cars"][car]
    r = rotation(after["rotation"])
    inertia = [
        MASS / 12 * ((2 * HALF[1]) ** 2 + (2 * HALF[2]) ** 2),
        MASS / 12 * ((2 * HALF[0]) ** 2 + (2 * HALF[2]) ** 2),
        MASS / 12 * ((2 * HALF[0]) ** 2 + (2 * HALF[1]) ** 2),
    ]
    dv = [a - b for a, b in zip(vec(after, "velocity"), vec(before, "velocity"))]
    dv[2] += gravity * TICK
    dw = [a - b for a, b in zip(vec(after, "angular_velocity"), vec(before, "angular_velocity"))]
    local = matvec(transpose(r), dw)
    torque = matvec(r, [inertia[i] * local[i] for i in range(3)])
    j = [MASS * x for x in dv]
    jj = dot(j, j)
    print(f"dv {[round(x, 1) for x in dv]}  dw {[round(x, 2) for x in dw]}")
    print(f"J {[round(x) for x in j]}  |J| {math.sqrt(jj):.0f}  direction {[round(x / math.sqrt(jj), 3) for x in j]}")
    cosine = dot(torque, j) / math.sqrt(max(dot(torque, torque) * jj, 1e-12))
    print(f"cos(torque, J) {cosine:.3f}  (0 for a single contact)")
    nearest = [x / jj for x in cross(j, torque)]
    print(f"line of action, closest to the centre of mass: car frame {[round(x, 1) for x in matvec(transpose(r), nearest)]}")


if __name__ == "__main__":
    main()
