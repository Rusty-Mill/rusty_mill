"""Air control with several axes held together (PARITY-PLAN workstream A/B gaps; the random
drives' first divergences are mostly in the air, several with yaw and roll together).

    python -I tools/rb_tape_bot/gen_air_combo.py

Writes experiments/aircombo_<name>.json: a level car at rest high in the air (z = 1000, so it
only falls for the 1 s of the tape), a held combination of pitch, yaw and roll, with and
without boost. The single-axis cases are already well modelled (`air_roll`, `air_yaw_pitch`);
this is the rest of the cube. Prints the names.
"""
import json
import pathlib

EXP = pathlib.Path(__file__).resolve().parent / "experiments"
TICKS = 120
# name: (pitch, yaw, roll, boost)
COMBOS = {
    "yaw_roll_same": (0, 1, 1, False),
    "yaw_roll_opp": (0, 1, -1, False),
    "pitch_yaw_roll": (1, 1, 1, False),
    "pitch_yaw_rollneg": (-1, 1, -1, False),
    "half_all": (0.5, -0.5, 0.5, False),
    "yaw_roll_same_boost": (0, 1, 1, True),
    "pitch_yaw_roll_boost": (1, 1, -1, True),
    "pitch_roll": (1, 0, 1, False),
    "pitch_rollneg": (1, 0, -1, False),
    "pitch_rollneg_boost": (1, 0, -1, True),
    "pitch_rollneg_moving": (1, 0, -1, False),
    "pitch_roll_moving": (1, 0, 1, False),
    "jump_pitch_roll": (1, 0, 1, False),
    "jump_half_pitch_roll": (-0.5, 0, -0.5, False),
    "jump_pitch": (1, 0, 0, False),
    "jump_half_pitch_roll_opp": (-0.5, 0, 0.5, False),
    "late_pitch_roll": (1, 0, 1, False),
    "late_pitch": (1, 0, 0, False),
    "early_pitch": (1, 0, 0, False),
}
# Level car, 60 idle ticks first: does the first-0.2-s pitch silence belong to the tape start?
LATE = {"late_pitch_roll", "late_pitch"}
# Cases that start on the floor and jump first (the fuzz tape fuzz_722 pitched and rolled the
# car right after a ground jump and the game pitched a little).
JUMPED = {"jump_pitch_roll", "jump_half_pitch_roll", "jump_pitch", "jump_half_pitch_roll_opp"}
# Start state for the moving cases: yaw 45 degrees, nose 30 degrees up, 1000 uu/s forward,
# like the speed flip that first showed pitch surviving a held roll.
MOVING = {"pitch_rollneg_moving", "pitch_roll_moving"}


def tape(name: str, pitch: float, yaw: float, roll: float, boost: bool, moving: bool = False) -> dict:
    step = {"ticks": TICKS, "pitch": pitch, "yaw": yaw, "roll": roll}
    if boost:
        step["boost"] = True
    steps = [step]
    location = [0, 0, 1000]
    if name in LATE:
        steps = [{"ticks": 60}, step]
    if name in JUMPED:
        location = [0, 0, 17]
        steps = [{"ticks": 3, "jump": True}, step]
    return {
        "name": f"aircombo_{name}: level at z = 1000, pitch {pitch} yaw {yaw} roll {roll}{' with boost' if boost else ''}",
        "settle_ticks": 0,
        "car": {
            "location": location,
            "rotation": [0.5236, 0.7854, 0] if moving else [0, 0, 0],
            "velocity": [612, 612, 400] if moving else [0, 0, 0],
            "angular_velocity": [0, 0, 0],
            "boost": 100 if boost else 0,
        },
        "ball": {"location": [3500, 3500, 93.15], "velocity": [0, 0, 0]},
        "steps": steps,
    }


def main() -> None:
    names = []
    for key, (pitch, yaw, roll, boost) in COMBOS.items():
        name = f"aircombo_{key}"
        (EXP / f"{name}.json").write_text(json.dumps(tape(key, pitch, yaw, roll, boost, key in MOVING), indent=2) + chr(10), encoding="utf-8", newline=chr(10))
        names.append(name)
    print(",".join(names))


if __name__ == "__main__":
    main()
