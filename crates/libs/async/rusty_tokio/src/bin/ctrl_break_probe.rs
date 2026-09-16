// A tiny helper process for `tests/signal_windows.rs`. Cargo
// auto-discovers `src/bin/*.rs` as additional binary targets, each
// getting its own `CARGO_BIN_EXE_<name>` env var for integration tests
// to spawn -- no `Cargo.toml` registration needed.
//
// Listens for `CTRL_BREAK_EVENT` -- via *two* independent listeners, to
// also exercise `signal`'s "additive installation" contract (every
// `windows::ctrl_break()` call adds its own independent listener; one
// real event wakes all of them, not just the first, mirroring the Unix
// `signal` module's own documented contract) -- and prints a marker
// once both have fired, so the parent test (which spawns this with
// `CREATE_NEW_PROCESS_GROUP` and sends
// `GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, ..)` at this process's own
// group) can observe real OS-level console-control delivery end to end
// through this crate's actual `SetConsoleCtrlHandler` wiring -- not a
// mock. `CTRL_BREAK_EVENT` (unlike `CTRL_C_EVENT`) can be targeted at
// one specific process group rather than the whole console, which is
// what makes it safe to fire from an automated test without risking the
// test harness's own process alongside it.

#[cfg(windows)]
fn main() {
    let rt = rusty_tokio::Runtime::new().unwrap();
    rt.block_on(async {
        let mut first = rusty_tokio::signal::windows::ctrl_break().unwrap();
        let mut second = rusty_tokio::signal::windows::ctrl_break().unwrap();
        // Tell the parent both listeners are actually registered before
        // it sends the event -- avoids a race where the event fires
        // before anything here is listening for it yet.
        println!("READY");
        use std::io::Write;
        std::io::stdout().flush().unwrap();

        rusty_tokio::join!(first.recv(), second.recv());
        println!("CTRL_BREAK_RECEIVED");
        std::io::stdout().flush().unwrap();
    });
}

#[cfg(not(windows))]
fn main() {
    eprintln!("ctrl_break_probe is Windows-only");
    std::process::exit(1);
}
