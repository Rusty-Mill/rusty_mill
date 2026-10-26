//! `rusty-mux <socket> [block]`: attach this terminal to a rusty_mux block.
//! Without a block id it creates a session, window and block first. Ctrl-]
//! detaches; the block keeps running and its id is printed for reattaching.

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.is_empty() || args.len() > 2 {
        eprintln!("usage: rusty-mux <socket> [block]");
        return ExitCode::from(2);
    }
    match client::run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("rusty-mux: {err}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(not(target_os = "linux"))]
mod client {
    pub fn run(_args: &[std::ffi::OsString]) -> Result<(), String> {
        Err("the Phase 1 client runs on Linux only".into())
    }
}

#[cfg(target_os = "linux")]
mod client {
    use std::ffi::OsString;
    use std::io::Write;
    use std::net::Shutdown;
    use std::os::unix::net::UnixStream;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    use platform::term::Terminal;
    use platform_linux::LinuxTerminal;
    use rusty_mux_core::{BlockId, SessionId, Size, WindowId};
    use rusty_mux_proto::{Features, Message};
    use rusty_mux_server::checkpoint::Receiver;
    use rusty_mux_tui::{Conn, RESTORE, paint};
    use rusty_uuid::Uuid;

    /// Ctrl-]: detach.
    const DETACH: u8 = 0x1d;

    /// How often the input loop checks whether the output side has ended.
    const TICK: Duration = Duration::from_millis(100);

    /// Why the output side stopped.
    enum Ended {
        /// The block's program exited or the block was closed.
        Closed,
        /// The server went away, or the stream was shut down on detach.
        Disconnected,
        /// The local terminal stopped taking output.
        Stdout,
    }

    pub fn run(args: &[OsString]) -> Result<(), String> {
        let stream = UnixStream::connect(&args[0]).map_err(|e| format!("connecting: {e}"))?;
        let mut conn = Conn::new(stream.try_clone().map_err(|e| e.to_string())?);
        let mut term = LinuxTerminal::new();
        let win = term
            .window_size()
            .map_err(|e| format!("terminal size: {e:?}"))?;
        // A pty nobody sized reports 0x0; fall back to the classic 80x24.
        let size = Size::new(win.cols, win.rows)
            .or_else(|_| Size::new(80, 24))
            .map_err(|e| e.to_string())?;

        conn.send(&Message::Hello {
            features: Features::CHECKPOINT_V3,
            resume: None,
        })
        .map_err(|e| e.to_string())?;
        let mut req = Requests {
            conn: &mut conn,
            next: 1,
            held: Vec::new(),
        };
        req.welcome()?;
        let block = match args.get(1) {
            Some(id) => BlockId::from_uuid(parse_uuid(id)?),
            None => req.create(size)?,
        };
        req.call(|req| Message::Attach { req, block })?;
        let writer = req.call(|req| Message::AcquireLease { req, block }).is_ok();
        if writer && let Err(err) = req.call(|req| Message::Resize { req, block, size }) {
            eprintln!("rusty-mux: keeping the block's size: {err}");
        }

        let held = std::mem::take(&mut req.held);
        let input = stream.try_clone().map_err(|e| e.to_string())?;
        term.enter_raw().map_err(|e| format!("raw mode: {e:?}"))?;
        let done = Arc::new(AtomicBool::new(false));
        let output = {
            let done = done.clone();
            std::thread::spawn(move || {
                let ended = show(conn, held, block);
                done.store(true, Ordering::Release);
                ended
            })
        };
        let result = forward(&term, input, block, writer, &done);
        // Stop the output side before touching the terminal, so nothing the
        // block prints lands after the restore. Shutting the socket ends
        // its read; an error only means the server already closed it.
        let _ = stream.shutdown(Shutdown::Both);
        let ended = output.join();
        restore(&mut term);
        match (ended, &result) {
            (Ok(Ended::Closed), _) => println!("\r\nrusty-mux: block {block} closed"),
            (Ok(_), Ok(Stop::Detached)) => {
                println!("\r\nrusty-mux: detached from block {block}")
            }
            (Ok(Ended::Disconnected), _) => {
                println!("\r\nrusty-mux: lost the server; block {block} may still be running")
            }
            (Ok(Ended::Stdout), _) => eprintln!("rusty-mux: the terminal stopped taking output"),
            (Err(_), _) => eprintln!("rusty-mux: the output thread panicked"),
        }
        result.map(|_| ())
    }

    /// Why the input loop stopped.
    enum Stop {
        /// Ctrl-], end of keyboard input, or the server refused input.
        Detached,
        /// The output side ended first.
        OutputEnded,
    }

    /// Undoes the modes a block's program may have set on this terminal,
    /// then leaves raw mode.
    fn restore(term: &mut LinuxTerminal) {
        let mut stdout = std::io::stdout();
        if let Err(err) = stdout.write_all(RESTORE).and_then(|()| stdout.flush()) {
            eprintln!("rusty-mux: resetting the terminal: {err}");
        }
        if let Err(err) = term.leave_raw() {
            eprintln!("rusty-mux: restoring the terminal: {err:?}");
        }
    }

    struct Requests<'a> {
        conn: &'a mut Conn<UnixStream>,
        next: u64,
        /// Messages that arrived while waiting for a reply, such as the
        /// checkpoint that follows an attach, kept for [`show`].
        held: Vec<Message>,
    }

    impl Requests<'_> {
        fn welcome(&mut self) -> Result<(), String> {
            match self.conn.recv().map_err(|e| e.to_string())? {
                Message::Welcome { features, .. } if features.contains(Features::CHECKPOINT_V3) => {
                    Ok(())
                }
                Message::Welcome { .. } => Err("the server cannot send checkpoints".into()),
                other => Err(format!("expected Welcome, got {other:?}")),
            }
        }

        /// Sends a request and waits for its reply, skipping everything else.
        fn call(&mut self, make: impl FnOnce(u64) -> Message) -> Result<Option<Uuid>, String> {
            let req = self.next;
            self.next += 1;
            self.conn.send(&make(req)).map_err(|e| e.to_string())?;
            loop {
                match self.conn.recv().map_err(|e| e.to_string())? {
                    Message::Done { req: r, created } if r == req => return Ok(created),
                    Message::Failed { req: r, reason, .. } if r == req => return Err(reason),
                    other => self.held.push(other),
                }
            }
        }

        fn create(&mut self, size: Size) -> Result<BlockId, String> {
            let missing = || "the server did not return an id".to_string();
            let session = SessionId::from_uuid(
                self.call(|req| Message::CreateSession {
                    req,
                    name: "main".into(),
                })?
                .ok_or_else(missing)?,
            );
            let window = WindowId::from_uuid(
                self.call(|req| Message::CreateWindow {
                    req,
                    session,
                    name: "main".into(),
                })?
                .ok_or_else(missing)?,
            );
            let block = self
                .call(|req| Message::CreateBlock { req, window, size })?
                .ok_or_else(missing)?;
            Ok(BlockId::from_uuid(block))
        }
    }

    /// Server to screen: assemble the checkpoint and paint its grid at
    /// Ready, then pass raw output through until the block closes or the
    /// connection ends.
    fn show(mut conn: Conn<UnixStream>, held: Vec<Message>, block: BlockId) -> Ended {
        // Locked per write: the main thread prints the detach line when it is done.
        let mut stdout = std::io::stdout();
        let mut held = held.into_iter();
        let mut receiver = Some(Receiver::default());
        while let Some(message) = held.next().or_else(|| conn.recv().ok()) {
            let bytes = match message {
                Message::Checkpoint {
                    block: b,
                    offset,
                    total,
                    data,
                } if b == block => match receiver.as_mut().map(|r| r.push(offset, total, &data)) {
                    Some(Ok(())) => continue,
                    Some(Err(err)) => fail(&format!("bad checkpoint: {err}")),
                    None => fail("checkpoint after ready"),
                },
                Message::Ready { block: b } if b == block => {
                    match receiver.take().map(Receiver::finish) {
                        Some(Ok((grid, _parser))) => paint(&grid),
                        Some(Err(err)) => fail(&format!("bad checkpoint: {err}")),
                        None => fail("ready twice"),
                    }
                }
                Message::Output { block: b, data } if b == block => data,
                Message::Event {
                    event: rusty_mux_core::Event::BlockClosed { block: b },
                    ..
                } if b == block => {
                    return Ended::Closed;
                }
                _ => continue,
            };
            if stdout
                .write_all(&bytes)
                .and_then(|()| stdout.flush())
                .is_err()
            {
                return Ended::Stdout;
            }
        }
        Ended::Disconnected
    }

    /// Keyboard to server, until Ctrl-], the end of keyboard input, or the
    /// output side ending (`done`), which it checks every [`TICK`].
    fn forward(
        term: &LinuxTerminal,
        stream: UnixStream,
        block: BlockId,
        writer: bool,
        done: &AtomicBool,
    ) -> Result<Stop, String> {
        let mut conn = Conn::new(stream);
        let mut buf = [0u8; 4096];
        let mut req = 1_000_000;
        loop {
            if done.load(Ordering::Acquire) {
                return Ok(Stop::OutputEnded);
            }
            let ready = term
                .poll_readable(Some(TICK))
                .map_err(|e| format!("waiting for the keyboard: {e:?}"))?;
            if !ready {
                continue;
            }
            let n = term
                .read_chunk(&mut buf)
                .map_err(|e| format!("reading the keyboard: {e:?}"))?;
            if n == 0 {
                return Ok(Stop::Detached);
            }
            let keys = &buf[..n];
            let (keys, detach) = match keys.iter().position(|&b| b == DETACH) {
                Some(at) => (&keys[..at], true),
                None => (keys, false),
            };
            if writer && !keys.is_empty() {
                req += 1;
                let input = Message::Input {
                    req,
                    block,
                    data: keys.to_vec(),
                };
                if conn.send(&input).is_err() {
                    return Ok(Stop::Detached);
                }
            }
            if detach {
                return Ok(Stop::Detached);
            }
        }
    }

    /// A line for the user about a broken attach.
    fn fail(why: &str) -> Vec<u8> {
        format!("\r\nrusty-mux: {why}\r\n").into_bytes()
    }

    fn parse_uuid(text: &OsString) -> Result<Uuid, String> {
        let text = text.to_str().ok_or("block id is not UTF-8")?;
        text.parse().map_err(|e| format!("block id: {e}"))
    }
}
