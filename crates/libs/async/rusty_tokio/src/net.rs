//! tokio's `net` paths for this crate's sockets (#897). The types live in
//! [`crate::io`]; this module re-exports them under the names tokio uses,
//! so `use tokio::net::{TcpListener, tcp::OwnedReadHalf}` migrates with
//! only the crate renamed. Every item here is the same type as its `io`
//! counterpart, not a wrapper.
//!
//! Names follow tokio's nesting: the TCP halves under [`tcp`], the Unix
//! halves, address and pipe types under [`unix`], where `io` prefixes
//! them (`UnixReadHalf`, `UnixSocketAddr`, `PipeSender`) to keep its flat
//! namespace unambiguous.

pub use crate::io::{lookup_host, TcpListener, TcpSocket, TcpStream, ToSocketAddrs, UdpSocket};
#[cfg(unix)]
pub use crate::io::{UnixDatagram, UnixSocket};
#[cfg(any(unix, windows))]
pub use crate::io::{UnixListener, UnixStream};

/// TCP stream halves, as `tokio::net::tcp`.
pub mod tcp {
    pub use crate::io::{OwnedReadHalf, OwnedWriteHalf, ReadHalf, ReuniteError, WriteHalf};
}

/// Unix socket halves, address, credentials and pipes, as `tokio::net::unix`.
#[cfg(any(unix, windows))]
pub mod unix {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub use crate::io::UCred;
    #[cfg(unix)]
    pub use crate::io::{gid_t, pid_t, uid_t};
    pub use crate::io::{
        OwnedUnixReadHalf as OwnedReadHalf, OwnedUnixWriteHalf as OwnedWriteHalf,
        UnixReadHalf as ReadHalf, UnixReuniteError as ReuniteError, UnixSocketAddr as SocketAddr,
        UnixWriteHalf as WriteHalf,
    };

    /// Unix pipes, as `tokio::net::unix::pipe`.
    #[cfg(unix)]
    pub mod pipe {
        pub use crate::io::{
            pipe, PipeOpenOptions as OpenOptions, PipeReceiver as Receiver, PipeSender as Sender,
        };
    }
}
