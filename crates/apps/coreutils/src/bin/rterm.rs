//! Thin CLI over `coreutils::term_report`, wired to the real backend —
//! the Terminal surface's end-to-end prover (extraction map D9).
//!
//! `rterm` prints the tty report; `rterm --raw-probe` additionally
//! enters raw mode, polls stdin readable, reads one batched chunk via
//! `Terminal::read_chunk`, restores, and reports it (interactive use
//! only — it refuses cleanly when stdin is not a tty); `rterm
//! --echo-probe` toggles echo off, reads a line, restores it, all
//! through `Terminal::set_echo` (the password-prompt shape, slice 2).

fn main() -> std::process::ExitCode {
    let args: Vec<_> = std::env::args_os().collect();
    let raw_probe = args.iter().any(|a| a == "--raw-probe");
    let echo_probe = args.iter().any(|a| a == "--echo-probe");
    let mut term = coreutils::native::terminal();
    print!("{}", coreutils::term_report::report(term.as_ref()));

    if raw_probe {
        let got = coreutils::term_report::with_raw(term.as_mut(), |raw| {
            if !raw.poll_readable(None)? {
                return Err(platform::error::PlatformError::new(
                    platform::error::ErrorKind::Other,
                    platform::error::OsCode::None,
                    "poll_readable",
                ));
            }
            let mut buf = [0u8; 64];
            let n = raw.read_chunk(&mut buf)?;
            Ok(buf[..n].to_vec())
        });
        match got {
            Ok(bytes) => println!("raw probe: read {} byte(s): {bytes:02x?}", bytes.len()),
            Err(e) => {
                eprintln!("rterm: raw probe: {e}");
                return std::process::ExitCode::FAILURE;
            }
        }
    }

    if echo_probe {
        let prev = match term.set_echo(false) {
            Ok(prev) => prev,
            Err(e) => {
                eprintln!("rterm: echo probe: {e}");
                return std::process::ExitCode::FAILURE;
            }
        };
        let mut line = String::new();
        let _ = std::io::stdin().read_line(&mut line);
        term.set_echo(prev).ok();
        println!("echo probe: read {} byte(s), echo restored", line.len());
    }

    std::process::ExitCode::SUCCESS
}
