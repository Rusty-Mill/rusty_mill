#![no_main]
//! Fuzz `Pake::update` with arbitrary peer messages across every curve.
//! Malformed JSON / off-curve points must error, never panic.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    for curve in rusty_croc::pake::available_curves() {
        for role in [0u8, 1u8] {
            if let Ok(mut p) = rusty_croc::pake::Pake::init_curve(b"weak-secret", role, curve) {
                let _ = p.update(data);
            }
        }
    }
});
