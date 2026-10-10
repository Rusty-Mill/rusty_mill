//! A small stdio server for `tests/interop.rs`, which drives it with the
//! `rmcp` client. `INTEROP_CANCEL_FILE`, when set, is created once a `wait`
//! call observes its cancellation. The server itself is `tests/common/fixture.rs`.

#[path = "../tests/common/fixture.rs"]
mod fixture;

use rusty_mcp_server::serve_stdio;
use std::sync::Arc;

fn main() -> std::io::Result<()> {
    let cancel_file = std::env::var_os("INTEROP_CANCEL_FILE");
    let server = fixture::interop_server(move || {
        if let Some(path) = &cancel_file {
            let _ = std::fs::write(path, "cancelled");
        }
    });
    serve_stdio(Arc::new(server))
}
