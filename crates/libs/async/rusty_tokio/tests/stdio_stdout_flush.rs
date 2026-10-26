//! Deliberately the *only* test in this file: it redirects the process's
//! fd 1, so it must run alone in its process (see
//! `tests/stdio_stdin_redirect.rs`'s module docs).
//!
//! `std::io::Stdout` is line-buffered, so bytes after the last newline
//! (an LSP frame's JSON body, for one) stay buffered until something
//! flushes them. `flush` on `rusty_tokio::io::stdout()` must deliver them
//! while the process is still running.
#![cfg(unix)]

use rusty_tokio::io::AsyncWriteExt;
use rusty_tokio::Runtime;

#[test]
fn flush_delivers_bytes_after_the_last_newline() {
    const REAL_STDOUT_FD: i32 = 1;
    let mut pipe_fds = [0i32; 2];
    assert_eq!(
        unsafe { libc::pipe(pipe_fds.as_mut_ptr()) },
        0,
        "pipe() failed"
    );
    let (read_fd, write_fd) = (pipe_fds[0], pipe_fds[1]);
    let saved_stdout = unsafe { libc::dup(REAL_STDOUT_FD) };
    assert!(saved_stdout >= 0, "dup(stdout) failed");
    assert_eq!(
        unsafe { libc::dup2(write_fd, REAL_STDOUT_FD) },
        REAL_STDOUT_FD
    );
    unsafe { libc::close(write_fd) };

    let payload = b"header\r\n\r\n{\"id\":1}";
    let rt = Runtime::new().unwrap();
    rt.block_on(async {
        let mut out = rusty_tokio::io::stdout();
        out.write_all(payload).await.unwrap();
        out.flush().await.unwrap();
    });

    // Read back while stdout is still redirected and nothing else will
    // flush it: only `flush` above can have delivered the tail.
    let mut captured = Vec::new();
    let mut chunk = [0u8; 256];
    while captured.len() < payload.len() {
        let mut pfd = libc::pollfd {
            fd: read_fd,
            events: libc::POLLIN,
            revents: 0,
        };
        if unsafe { libc::poll(&mut pfd, 1, 5_000) } <= 0 {
            break;
        }
        let n = unsafe { libc::read(read_fd, chunk.as_mut_ptr().cast(), chunk.len()) };
        if n <= 0 {
            break;
        }
        captured.extend_from_slice(&chunk[..n as usize]);
    }
    unsafe {
        libc::dup2(saved_stdout, REAL_STDOUT_FD);
        libc::close(saved_stdout);
        libc::close(read_fd);
    }
    assert_eq!(
        captured, payload,
        "the bytes after the last newline were delivered"
    );
}
