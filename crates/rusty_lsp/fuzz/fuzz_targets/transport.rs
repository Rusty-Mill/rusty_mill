//! Fuzz the frame parser: arbitrary bytes must never panic, and must
//! either yield a message, a clean EOF, or an error.
#![no_main]

use libfuzzer_sys::fuzz_target;
use std::sync::OnceLock;
use tokio::runtime::Runtime;

fn runtime() -> &'static Runtime {
    static RT: OnceLock<Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
    })
}

fuzz_target!(|data: &[u8]| {
    runtime().block_on(async {
        let mut reader = rusty_lsp::transport::buffered(data);
        // Read until the stream is exhausted or errors; no panics allowed.
        while let Ok(Some(_message)) = rusty_lsp::transport::read_message(&mut reader).await {}
    });
});
