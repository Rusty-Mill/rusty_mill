//! The game's 8-bit input quantization.
//!
//! Every analog input is clamped to `[-1, 1]` and round-tripped through one
//! byte before the car sees it:
//! `f = v < 0 ? max(v * 128, -128) : min(v * 127, 127)`,
//! `byte = floor(f + 128 + 0.5)`, decoded as `(byte - 128) / 128` below 128 and
//! `(byte - 128) / 127` from 128 up. Negative values step by 1/128, positive by
//! 1/127, so `+0.5` becomes 64/127 = 0.5039 while `-0.5` stays `-0.5`.
//!
//! Confirmed on captures (`tools/rb_tape_bot/capture_checks.py`): the spin from a
//! held `+v` over a held `-v` on roll is 1.0080, 1.0080, 1.0079, 0.9967 and 0.9991
//! at v = 0.1, 0.3, 0.5, 0.7 and 0.9, the byte rule's prediction at every level
//! (the two signs round differently, so an absent or symmetric quantizer gives
//! 1.0000); 1.008 to 1.009 on pitch and yaw at 0.5. It applies to throttle, steer,
//! pitch, yaw and roll; the button bits are untouched.

use rb_domain::ControllerInput;

/// One analog value after the game's byte round trip. Out-of-range values clamp
/// to `[-1, 1]`; NaN reads as 0.
pub fn quantize_analog(value: f32) -> f32 {
    if value.is_nan() {
        return 0.0;
    }
    let scaled = if value < 0.0 {
        (value * 128.0).max(-128.0)
    } else {
        (value * 127.0).min(127.0)
    };
    let byte = (scaled + 128.0 + 0.5).floor();
    if byte < 128.0 {
        (byte - 128.0) / 128.0
    } else {
        (byte - 128.0) / 127.0
    }
}

/// `input` as the car receives it: every analog axis quantized, buttons and
/// missing sticks (`None`) unchanged.
pub fn quantize_input(input: ControllerInput) -> ControllerInput {
    ControllerInput {
        throttle: quantize_analog(input.throttle),
        steer: quantize_analog(input.steer),
        pitch: input.pitch.map(quantize_analog),
        yaw: input.yaw.map(quantize_analog),
        roll: input.roll.map(quantize_analog),
        ..input
    }
}
