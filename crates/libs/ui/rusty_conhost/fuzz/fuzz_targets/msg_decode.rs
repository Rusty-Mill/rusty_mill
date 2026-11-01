//! Raw bytes into every ConDrv packet decoder: a client controls each
//! size, offset and API number (design §6.1).

#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| rusty_conhost::fuzz::decode(data));
