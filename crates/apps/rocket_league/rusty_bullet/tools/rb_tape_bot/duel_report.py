"""Scores a batch of two-car tapes around their first car-car contact.

    python -I tools/rb_tape_bot/duel_report.py replays/batch_<stamp> [contact_distance_uu]

For every `duel_*_run1.jsonl` (see `gen_duel_fuzz.py`): the first recorded frame where the two
cars' origins are closer than `contact_distance_uu` (default 125), each car's position error
(port fed the recorded input, aligned at lag 0 / offset 0 as the throttle onsets of these
tapes say) just before, 5 ticks after and 40 ticks after it, the cars' teams and the recorded
speeds into the contact. A contact that leaves both errors small is a good bump model; a jump
in either is the case to open with `RB_STATES=1 RB_OTHERS=1 rb-verify`.
"""
import glob
import json
import math
import os
import pathlib
import re
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
VERIFY = ROOT / "target" / "release" / "rb-verify.exe"
STATE = re.compile(
    r"car (\d) tick\s+(\d+): rec pos \(\s*(-?[\d.]+),\s*(-?[\d.]+),\s*(-?[\d.]+)\).*?pred pos \(\s*(-?[\d.]+),\s*(-?[\d.]+),\s*(-?[\d.]+)\)"
)


def first_contact(frames: list, distance: float):
    for i, frame in enumerate(frames[14:]):
        if len(frame["cars"]) < 2:
            continue
        a, b = frame["cars"][0], frame["cars"][1]
        pa, pb = a["position"], b["position"]
        if math.dist((pa["x"], pa["y"], pa["z"]), (pb["x"], pb["y"], pb["z"])) < distance:
            speed = lambda c: math.sqrt(sum(c["velocity"][k] ** 2 for k in "xyz")) if isinstance(c["velocity"], dict) else 0.0
            return i, speed(a), speed(b)
    return None


def main() -> int:
    args = sys.argv[1:]
    distance = 125.0
    if len(args) > 1:
        distance = float(args.pop())
    if not args:
        print(__doc__)
        return 2
    contacts = 0
    for capture in sorted(glob.glob(os.path.join(args[0], "duel_*_run1.jsonl"))):
        name = pathlib.Path(capture).name[:-11]
        frames = [json.loads(line) for line in open(capture, encoding="utf-8")]
        scenario = json.load(open(ROOT / "tools" / "rb_tape_bot" / "experiments" / f"{name}.json", encoding="utf-8"))
        contact = first_contact(frames, distance)
        if contact is None:
            print(f"{name}: no contact")
            continue
        contacts += 1
        tick, speed_a, speed_b = contact
        env = {**os.environ, "RB_OTHERS": "1", "RB_STATES": "1", "RB_FORCE_ALIGN": "0,0"}
        out = subprocess.run(
            [str(VERIFY), "--scenario", str(ROOT / "tools" / "rb_tape_bot" / "experiments" / f"{name}.json"), "--against", capture, "--recorded-inputs", "1"],
            capture_output=True, text=True, env=env,
        ).stdout
        errors = {}
        for line in out.splitlines():
            m = STATE.match(line)
            if m:
                errors[(int(m.group(1)), int(m.group(2)))] = math.dist(
                    [float(m.group(k)) for k in (3, 4, 5)], [float(m.group(k)) for k in (6, 7, 8)]
                )

        def err(car: int, at: int) -> str:
            value = errors.get((car, max(at, 0)))
            return "  -- " if value is None else f"{value:5.1f}"

        team = scenario["others"][0]["team"]
        print(
            f"{name}: contact tick {tick}, team {team}, car0 err {err(0, tick - 3)} -> {err(0, tick + 5)} -> {err(0, tick + 40)}, "
            f"car1 err {err(1, tick - 3)} -> {err(1, tick + 5)} -> {err(1, tick + 40)}"
        )
    print(f"{contacts} of {len(glob.glob(os.path.join(args[0], 'duel_*_run1.jsonl')))} tapes with a contact")
    return 0


if __name__ == "__main__":
    sys.exit(main())
