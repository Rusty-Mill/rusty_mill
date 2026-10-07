"""Generates the mechanics probe scenarios into tools/rb_tape_bot/experiments/.

    python -I tools/rb_tape_bot/gen_probes.py

Round 1 (driving, air, wall, ball, car-ball) and round 2 (more powerslides,
wall rides, boost in the air). Run them with
`run_batch.ps1 -Scenarios probe_accel,probe_brake,...` and score with
`rb-verify --scenario ... --against ... --recorded-inputs`.
"""
import json
import pathlib

OUT = str(pathlib.Path(__file__).resolve().parent / "experiments") + "/"
FAR_BALL = {"location": [3500, 3500, 93.15], "velocity": [0, 0, 0]}
FACE_Y = [0, 1.5708, 0]
names = []


def car(loc, vel=(0, 0, 0), rot=FACE_Y, boost=100, spin=(0, 0, 0)):
    return {
        "location": list(loc),
        "rotation": list(rot),
        "velocity": list(vel),
        "angular_velocity": list(spin),
        "boost": boost,
    }


def add(name, desc, c, steps, ball=None, settle=60):
    d = {
        "name": f"{name}: {desc}",
        "settle_ticks": settle,
        "car": c,
        "ball": ball or FAR_BALL,
        "steps": steps,
    }
    with open(OUT + name + ".json", "w", newline="\n") as f:
        f.write(json.dumps(d, indent=2) + "\n")
    names.append(name)


def st(ticks, **inp):
    return dict({"ticks": ticks}, **inp)


# Driving
add("probe_accel", "full throttle from rest, no boost", car((0, -4500, 17), boost=0), [st(400, throttle=1)])
add("probe_accel_boost", "full throttle and boost from rest", car((0, -4500, 17)), [st(360, throttle=1, boost=True)])
add("probe_coast", "1400 uu/s, no input (coasting drag)", car((0, -4500, 17), vel=(0, 1400, 0), boost=0), [st(300)], settle=0)
add("probe_brake", "1400 uu/s, brake (throttle -1) then release", car((0, -4500, 17), vel=(0, 1400, 0), boost=0), [st(150, throttle=-1), st(100)], settle=0)
add("probe_reverse", "full reverse from rest", car((0, -1000, 17), boost=0), [st(360, throttle=-1)])
add("probe_turn_slow", "throttle, full right steer from rest", car((0, -1500, 17), boost=0), [st(420, throttle=1, steer=1)])
add("probe_turn_fast", "1400 uu/s, throttle, half left steer", car((2000, -4500, 17), vel=(0, 1400, 0), boost=0), [st(300, throttle=1, steer=-0.5)], settle=0)
add("probe_powerslide", "1400 uu/s, handbrake with full right steer", car((-1500, -4500, 17), vel=(0, 1400, 0), boost=0), [st(180, throttle=1, steer=1, handbrake=True), st(60)], settle=0)
add("probe_turn_boost", "boosting at 2000 uu/s, full left steer", car((2500, -4500, 17), vel=(0, 2000, 0)), [st(240, throttle=1, boost=True, steer=-1)], settle=0)
# Air
for nm, kw in (("back", {"pitch": 1}), ("side", {"roll": 1}), ("diag", {"pitch": -1, "roll": -1})):
    add(f"probe_dodge_{nm}", f"800 uu/s, jump, {nm} dodge after 20 ticks", car((0, -4500, 17), vel=(0, 800, 0), boost=0),
        [st(10, jump=True), st(10), st(1, jump=True, **kw), st(150)], settle=0)
add("probe_air_roll", "jump, then full air roll right", car((0, -2000, 17), boost=0), [st(20, jump=True), st(90, roll=1), st(100)])
add("probe_air_yaw_pitch", "jump, pitch up then yaw left", car((0, -2000, 17), boost=0), [st(20, jump=True), st(60, pitch=1), st(60, yaw=-1), st(100)])
add("probe_boost_air", "jump, then boost 1.25 s with nose pitched up", car((0, -4000, 17)), [st(20, jump=True), st(15, pitch=-1), st(150, boost=True), st(100)])
# Wall
add("probe_wall_ride", "1200 uu/s toward the side wall at 20 degrees, throttle", car((3300, -3000, 17), vel=(410, 1128, 0), rot=[0, 1.2217, 0], boost=0), [st(360, throttle=1)], settle=0)
# Ball
ball_car = car((-3500, -4000, 17), boost=0)
add("probe_ball_drop", "ball dropped from 1500 uu", ball_car, [st(420)], ball={"location": [0, 0, 1500], "velocity": [0, 0, 0]})
add("probe_ball_drop_spin", "ball dropped with 6 rad/s spin", ball_car, [st(420)], ball={"location": [1000, 500, 1000], "velocity": [0, 0, 0], "angular_velocity": [0, 6, 0]})
add("probe_ball_wall", "ball into the side wall", ball_car, [st(300)], ball={"location": [-2000, 0, 500], "velocity": [3500, 500, 200]})
add("probe_ball_corner", "ball into a corner", ball_car, [st(300)], ball={"location": [3000, 3500, 300], "velocity": [2500, 2500, 0]})
add("probe_ball_ceiling", "ball up into the ceiling", ball_car, [st(360)], ball={"location": [0, 0, 1500], "velocity": [0, 0, 3000]})
add("probe_ball_goal", "ball into the goal and net", ball_car, [st(360)], ball={"location": [500, 3000, 300], "velocity": [200, 3500, 0]})
add("probe_ball_roll", "ball rolling on the floor at 1500 uu/s", ball_car, [st(420)], ball={"location": [0, 0, 93.15], "velocity": [1500, 0, 0]})
add("probe_ball_spin_roll", "ball sliding with spin", ball_car, [st(420)], ball={"location": [0, 0, 93.15], "velocity": [1500, 0, 0], "angular_velocity": [0, -8, 0]})
# Car-ball
rest_ball = {"location": [0, -1000, 93.15], "velocity": [0, 0, 0]}
add("probe_hit_ground", "car drives into a resting ball, throttle only", car((0, -3000, 17), boost=0), [st(300, throttle=1)], ball=rest_ball)
add("probe_hit_boost", "car drives into a resting ball, boosting", car((0, -4500, 17)), [st(300, throttle=1, boost=True)], ball=rest_ball)
add("probe_hit_offset", "car clips a resting ball off-centre", car((100, -3000, 17), boost=0), [st(300, throttle=1)], ball=rest_ball)

print(len(names), "scenarios")


# ---- round 2: more variants of the gaps found in round 1
names2 = []
_old_add = add


def add2(name, *a, **k):
    _old_add(name, *a, **k)
    names2.append(name)


add2("probe_ps_straight", "1500 uu/s, handbrake with no steer", car((0, -4500, 17), vel=(0, 1500, 0), boost=0), [st(120, throttle=1, handbrake=True), st(60)], settle=0)
add2("probe_ps_half_left", "1800 uu/s, handbrake with half left steer", car((2000, -4500, 17), vel=(0, 1800, 0), boost=0), [st(150, throttle=1, steer=-0.5, handbrake=True), st(60)], settle=0)
add2("probe_ps_boost", "1200 uu/s, handbrake, boost and full right steer", car((-2000, -4500, 17), vel=(0, 1200, 0)), [st(150, throttle=1, boost=True, steer=1, handbrake=True), st(60)], settle=0)
add2("probe_ps_slow", "600 uu/s, handbrake and full right steer", car((-500, -4500, 17), vel=(0, 600, 0), boost=0), [st(150, throttle=1, steer=1, handbrake=True), st(60)], settle=0)
add2("probe_ps_release", "handbrake right steer 60 ticks, then release with throttle", car((-1500, -4500, 17), vel=(0, 1400, 0), boost=0), [st(60, throttle=1, steer=1, handbrake=True), st(150, throttle=1)], settle=0)
add2("probe_ps_left_slow_release", "900 uu/s left handbrake 40 ticks then straight", car((1500, -4500, 17), vel=(0, 900, 0), boost=0), [st(40, throttle=1, steer=-1, handbrake=True), st(120, throttle=1)], settle=0)
add2("probe_wall_ride_45", "1200 uu/s into the side wall at 45 degrees", car((2800, -3000, 17), vel=(850, 850, 0), rot=[0, 0.7854, 0], boost=0), [st(360, throttle=1)], settle=0)
add2("probe_wall_ride_slow", "700 uu/s into the side wall at 20 degrees", car((3600, -3000, 17), vel=(240, 660, 0), rot=[0, 1.2217, 0], boost=0), [st(360, throttle=1)], settle=0)
add2("probe_wall_ride_boost", "boosting at 1800 uu/s into the side wall at 30 degrees", car((2900, -3500, 17), vel=(900, 1559, 0), rot=[0, 1.0472, 0]), [st(300, throttle=1, boost=True)], settle=0)
add2("probe_wall_ride_steer", "on the wall, then steer right", car((3300, -3000, 17), vel=(410, 1128, 0), rot=[0, 1.2217, 0], boost=0), [st(200, throttle=1), st(120, throttle=1, steer=1)], settle=0)
add2("probe_boost_air_flat", "jump, boost 1.25 s with no pitch input", car((0, -4000, 17)), [st(20, jump=True), st(150, boost=True), st(100)])
add2("probe_boost_air_pitchup", "jump, pitch up 0.5 s, boost 1.25 s", car((0, -4000, 17)), [st(20, jump=True), st(60, pitch=1), st(150, boost=True), st(100)])
add2("probe_boost_air_roll", "jump, roll right, boost 1.25 s", car((0, -4000, 17)), [st(20, jump=True), st(30, roll=1), st(150, boost=True), st(100)])
add2("probe_turn_boost_slow", "boosting from 800 uu/s with right steer", car((-2500, -4500, 17), vel=(0, 800, 0)), [st(240, throttle=1, boost=True, steer=1)], settle=0)
add2("probe_turn_half_boost", "boosting at 1800 uu/s with half right steer", car((-2500, -4500, 17), vel=(0, 1800, 0)), [st(240, throttle=1, boost=True, steer=0.5)], settle=0)
print(len(names2), "round-2 scenarios")


# ---- round 3: jumps, flips, landings, ceiling, fast dodges, ball on the car
names3 = []


def add3(name, *a, **k):
    _old_add(name, *a, **k)
    names3.append(name)


add3("probe_jump_short", "a one-tick jump press", car((0, -2000, 17), boost=0), [st(1, jump=True), st(120)])
add3("probe_jump_mid", "jump held 20 ticks", car((0, -2000, 17), boost=0), [st(20, jump=True), st(120)])
add3("probe_jump_long", "jump held 60 ticks (past the 0.2 s hold limit)", car((0, -2000, 17), boost=0), [st(60, jump=True), st(100)])
add3("probe_double_jump", "jump, then a second plain press 30 ticks later", car((0, -2000, 17), boost=0), [st(10, jump=True), st(20), st(1, jump=True), st(120)])
add3("probe_double_jump_moving", "jump at 1200 uu/s, plain second press 25 ticks later", car((0, -4500, 17), vel=(0, 1200, 0), boost=0), [st(10, jump=True), st(15), st(1, jump=True), st(120)], settle=0)
add3("probe_flip_cancel", "forward dodge cancelled at once with pitch back", car((0, -4500, 17), vel=(0, 1000, 0), boost=0), [st(10, jump=True), st(10), st(1, jump=True, pitch=-1), st(8, pitch=1), st(120)], settle=0)
add3("probe_flip_air_roll", "dodge back then air roll left", car((0, -4500, 17), vel=(0, 1000, 0), boost=0), [st(10, jump=True), st(10), st(1, jump=True, pitch=1), st(40, roll=-1), st(120)], settle=0)
add3("probe_dodge_fast_fwd", "1800 uu/s boosting forward dodge", car((0, -4800, 17), vel=(0, 1800, 0)), [st(10, jump=True, boost=True), st(10, boost=True), st(1, jump=True, pitch=-1, boost=True), st(100, boost=True)], settle=0)
add3("probe_dodge_fast_side", "1800 uu/s side dodge", car((-1000, -4800, 17), vel=(0, 1800, 0), boost=0), [st(10, jump=True), st(10), st(1, jump=True, roll=1), st(100)], settle=0)
add3("probe_land_wheels", "drop from 500 uu onto the wheels", car((0, -2000, 517), rot=[0, 1.5708, 0], boost=0), [st(240)], settle=0)
add3("probe_land_tilted", "drop from 400 uu nose-down 30 degrees with spin", car((0, -2000, 417), rot=[-0.52, 1.5708, 0.3], spin=(0.5, 0, 1.0), boost=0), [st(240)], settle=0)
add3("probe_wall_land", "jump toward the side wall and land on it", car((3600, -2000, 17), vel=(600, 600, 0), rot=[0, 0.7854, 0], boost=0), [st(20, jump=True, throttle=1), st(200, throttle=1)], settle=0)
add3("probe_ceiling", "boost upward into the ceiling", car((0, -2000, 17)), [st(20, jump=True, boost=True), st(60, boost=True, pitch=-1), st(240, boost=True)], settle=60)
add3("probe_roof_drop", "ball dropped on the roof of a stationary car", car((0, -2000, 17), boost=0), [st(300)], ball={"location": [0, -2000, 400], "velocity": [0, 0, -1]})
add3("probe_nose_hit_glancing", "car clips the ball at a diagonal", car((-300, -3000, 17), vel=(200, 1000, 0), rot=[0, 1.37, 0], boost=0), [st(240, throttle=1)], ball={"location": [0, -1500, 93.15], "velocity": [0, 0, 0]}, settle=0)
add3("probe_ball_bounce_car_side", "car drives into a rolling ball from the side", car((-2000, -2000, 17), vel=(1000, 0, 0), rot=[0, 0.0, 0], boost=0), [st(240, throttle=1)], ball={"location": [0, -2000, 93.15], "velocity": [0, 400, 0]}, settle=0)
print(len(names3), "round-3 scenarios")
