//! tokio's module paths resolve here under their tokio spelling (#897,
//! gap G1 in `docs/research/ASYNC-RUNTIME-INVENTORY.md`), so a migration
//! renames the crate and leaves its `use` lines alone. Each alias is
//! checked to be the same item as this crate's own path, not a lookalike.

use std::any::TypeId;

/// Both paths name the same type.
macro_rules! same_type {
    ($tokio:ty, $own:ty) => {
        assert_eq!(TypeId::of::<$tokio>(), TypeId::of::<$own>());
    };
}

#[test]
fn net_matches_io() {
    use rusty_tokio::{io, net};
    same_type!(net::TcpListener, io::TcpListener);
    same_type!(net::TcpSocket, io::TcpSocket);
    same_type!(net::TcpStream, io::TcpStream);
    same_type!(net::UdpSocket, io::UdpSocket);
    same_type!(net::tcp::OwnedReadHalf, io::OwnedReadHalf);
    same_type!(net::tcp::OwnedWriteHalf, io::OwnedWriteHalf);
    same_type!(net::tcp::ReadHalf<'static>, io::ReadHalf<'static>);
    same_type!(net::tcp::WriteHalf<'static>, io::WriteHalf<'static>);
    same_type!(net::tcp::ReuniteError, io::ReuniteError);
    // An `async fn`: resolving the path is the check; the future is never polled.
    drop(net::lookup_host("127.0.0.1:0"));
}

#[cfg(unix)]
#[test]
fn net_unix_matches_io() {
    use rusty_tokio::{io, net};
    same_type!(net::UnixListener, io::UnixListener);
    same_type!(net::UnixStream, io::UnixStream);
    same_type!(net::UnixDatagram, io::UnixDatagram);
    same_type!(net::UnixSocket, io::UnixSocket);
    same_type!(net::unix::OwnedReadHalf, io::OwnedUnixReadHalf);
    same_type!(net::unix::OwnedWriteHalf, io::OwnedUnixWriteHalf);
    same_type!(net::unix::ReadHalf<'static>, io::UnixReadHalf<'static>);
    same_type!(net::unix::WriteHalf<'static>, io::UnixWriteHalf<'static>);
    same_type!(net::unix::ReuniteError, io::UnixReuniteError);
    same_type!(net::unix::SocketAddr, io::UnixSocketAddr);
    same_type!(net::unix::uid_t, io::uid_t);
    same_type!(net::unix::gid_t, io::gid_t);
    same_type!(net::unix::pid_t, io::pid_t);
    same_type!(net::unix::pipe::Sender, io::PipeSender);
    same_type!(net::unix::pipe::Receiver, io::PipeReceiver);
    same_type!(net::unix::pipe::OpenOptions, io::PipeOpenOptions);
    let _: fn() -> std::io::Result<(io::PipeSender, io::PipeReceiver)> = net::unix::pipe::pipe;
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn net_unix_ucred_matches_io() {
    same_type!(rusty_tokio::net::unix::UCred, rusty_tokio::io::UCred);
}

#[test]
fn runtime_matches_root() {
    use rusty_tokio::runtime;
    same_type!(runtime::Builder, rusty_tokio::Builder);
    same_type!(runtime::EnterGuard, rusty_tokio::EnterGuard);
    same_type!(runtime::Handle, rusty_tokio::Handle);
    same_type!(runtime::Id, rusty_tokio::Id);
    same_type!(runtime::LocalOptions, rusty_tokio::LocalOptions);
    same_type!(runtime::LocalRuntime, rusty_tokio::LocalRuntime);
    same_type!(runtime::Runtime, rusty_tokio::Runtime);
    same_type!(runtime::RuntimeFlavor, rusty_tokio::RuntimeFlavor);
    same_type!(runtime::RuntimeMetrics, rusty_tokio::RuntimeMetrics);
    same_type!(runtime::TryCurrentError, rusty_tokio::TryCurrentError);
}

#[cfg(unix)]
#[test]
fn signal_unix_matches_signal() {
    use rusty_tokio::signal;
    same_type!(signal::unix::Signal, signal::Signal);
    same_type!(signal::unix::SignalKind, signal::SignalKind);
    let _: fn(signal::SignalKind) -> std::io::Result<signal::Signal> = signal::unix::signal;
}

#[cfg(unix)]
#[test]
fn io_unix_matches_io() {
    use rusty_tokio::io;
    same_type!(
        io::unix::AsyncFd<std::net::TcpStream>,
        io::AsyncFd<std::net::TcpStream>
    );
    same_type!(
        io::unix::AsyncFdReadyGuard<'static, std::net::TcpStream>,
        io::AsyncFdReadyGuard<'static, std::net::TcpStream>
    );
    same_type!(io::unix::TryIoError, io::TryIoError);
}

#[test]
fn task_spawn_blocking_runs_on_the_blocking_pool() {
    let rt = rusty_tokio::Runtime::new().unwrap();
    let n = rt.block_on(async { rusty_tokio::task::spawn_blocking(|| 21 * 2).await.unwrap() });
    assert_eq!(n, 42);
}

/// The paths tokio callers use most, spelled exactly as they write them.
#[test]
fn tokio_spellings_compile_and_run() {
    use rusty_tokio::net::{TcpListener, TcpStream};
    use rusty_tokio::runtime::{Builder, Handle};

    let rt = Builder::new_current_thread().build().unwrap();
    rt.block_on(async {
        assert!(Handle::try_current().is_some());
        let listener = TcpListener::bind("127.0.0.1:0".parse().unwrap()).unwrap();
        let addr = listener.local_addr().unwrap();
        let accept = rusty_tokio::spawn(async move { listener.accept().await.unwrap().0 });
        let client = TcpStream::connect(addr).await.unwrap();
        client.write_all(b"ok").await.unwrap();
        accept.await.unwrap();
    });
}
