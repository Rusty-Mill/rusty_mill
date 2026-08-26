//! `terminal_size()` / `with_echo_disabled` demo, for the pty test:
//! prints the reported terminal size, then brackets `with_echo_disabled`
//! (and, with `--panic`, a second call whose closure panics) so the test
//! can query termios independently in between. Each phase blocks on a
//! trigger file instead of a fixed sleep, so the test controls the
//! timing precisely regardless of scheduling — the same trick
//! `examples/notify.rs` uses.
//!
//!     cargo run --example terminal_facilities -- <trigger-dir> [--panic]

use rusty_lines::{terminal_size, with_echo_disabled};
use std::path::{Path, PathBuf};
use std::time::Duration;

fn wait_for(path: &Path) {
    while !path.exists() {
        std::thread::sleep(Duration::from_millis(20));
    }
    let _ = std::fs::remove_file(path);
}

fn main() {
    let mut args = std::env::args().skip(1);
    let dir: PathBuf = args
        .next()
        .expect("usage: terminal_facilities <trigger-dir> [--panic]")
        .into();
    let panic_too = args.next().as_deref() == Some("--panic");
    let gate = |name: &str| dir.join(name);

    match terminal_size() {
        Some((cols, rows)) => println!("size:{cols}x{rows}"),
        None => println!("size:none"),
    }

    println!("before-disable");
    wait_for(&gate("go1"));
    let _ = with_echo_disabled(|| {
        println!("during-disable");
        wait_for(&gate("go2"));
    });
    println!("after-disable");

    if panic_too {
        wait_for(&gate("go3"));
        let result = std::panic::catch_unwind(|| {
            let _ = with_echo_disabled(|| {
                println!("during-panic");
                wait_for(&gate("go4"));
                panic!("boom");
            });
        });
        println!("panicked:{}", result.is_err());
        println!("after-panic");
    }
}
