//! A script of packets, terminal input and control events into the
//! console, driven as the engine drives it; every effect is checked
//! against the limits of design §6.1 and §7.4.

#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| rusty_conhost::fuzz::packets(data));
