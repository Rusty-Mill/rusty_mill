//! Fuzz JSON-RPC message classification: arbitrary JSON must never panic,
//! and a successfully parsed message must re-serialize.
#![no_main]

use libfuzzer_sys::fuzz_target;
use rusty_lsp::jsonrpc::Message;

fuzz_target!(|data: &[u8]| {
    if let Ok(message) = serde_json::from_slice::<Message>(data) {
        let _ = serde_json::to_vec(&message).expect("parsed messages re-serialize");
    }
});
