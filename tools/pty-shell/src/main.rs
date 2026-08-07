//! Reference tool 3: opens an interactive PTY and spawns the host's
//! default shell in it. Exercises the PTY primitive only — no fs, no
//! captured (non-interactive) process spawn.
//!
//! This is a manual/interactive tool, not something CI runs headlessly:
//! it bridges the host terminal's stdin/stdout to the pty a byte at a
//! time. Exit the spawned shell (`exit` / Ctrl-D) ends the session — the
//! pty read side hits EOF right after, which is this tool's exit signal.

use std::io::{Read, Write};
use std::thread;

use compat::NativePtySession;
use contract::{Capabilities, PtySession};

fn main() -> anyhow::Result<()> {
    let caps = Capabilities::detect();
    eprintln!(
        "pty-shell: capabilities.pty_win32_input_mode={}",
        caps.pty_win32_input_mode
    );

    let session = NativePtySession;
    let spawn = session.spawn_shell(80, 24)?;
    let mut reader = spawn.reader;
    let mut writer = spawn.writer;
    let mut control = spawn.control;

    // Pump our stdin -> pty input on a detached background thread. It has
    // no clean way to unblock on shell exit (a blocking stdin read has no
    // signal to interrupt it), so the process simply exits out from under
    // it once the output side below observes EOF.
    thread::spawn(move || {
        let mut stdin = std::io::stdin();
        let mut buf = [0u8; 4096];
        loop {
            match stdin.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if writer.write_all(&buf[..n]).is_err() {
                        break;
                    }
                }
            }
        }
    });

    // Pump pty output -> our stdout on the main thread until EOF, which
    // the pty master delivers once the shell has exited and hung up.
    let mut buf = [0u8; 4096];
    loop {
        match reader.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                std::io::stdout().write_all(&buf[..n])?;
                std::io::stdout().flush()?;
            }
        }
    }

    let status = control.wait()?;
    std::process::exit(status);
}
