//! tokio-shaped signatures for the net constructors and the in-memory I/O
//! impls tokio code leans on (#897 follow-up): `bind`/`connect`/`send_to`
//! are `async` over `ToSocketAddrs`, Unix paths take `AsRef<Path>`, and
//! `Cursor`, `&[u8]`, `&mut T` and `Box<T>` implement the I/O traits.

use rusty_tokio::io::{
    AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncSeekExt, AsyncWriteExt, ReadBuf, TcpListener,
    TcpStream, UdpSocket,
};
use rusty_tokio::runtime::Builder;
use rusty_tokio::Runtime;
use std::io::{Cursor, SeekFrom};
use std::net::SocketAddr;

fn block_on<F: std::future::Future>(f: F) -> F::Output {
    Runtime::new().unwrap().block_on(f)
}

#[test]
fn tcp_bind_accepts_every_tokio_address_form() {
    block_on(async {
        let owned = String::from("127.0.0.1:0");
        let parsed: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let listeners = [
            TcpListener::bind("127.0.0.1:0").await.unwrap(),
            TcpListener::bind(&owned).await.unwrap(),
            TcpListener::bind(owned.clone()).await.unwrap(),
            TcpListener::bind(("127.0.0.1", 0)).await.unwrap(),
            TcpListener::bind(parsed).await.unwrap(),
            TcpListener::bind(&parsed).await.unwrap(),
            TcpListener::bind(&[parsed][..]).await.unwrap(),
        ];
        for listener in &listeners {
            assert!(listener.local_addr().unwrap().ip().is_loopback());
        }
    });
}

#[test]
fn tcp_bind_resolves_a_hostname() {
    block_on(async {
        let listener = TcpListener::bind("localhost:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let accept = rusty_tokio::spawn(async move { listener.accept().await.unwrap().0 });
        let client = TcpStream::connect(addr).await.unwrap();
        client.write_all(b"x").await.unwrap();
        accept.await.unwrap();
    });
}

#[test]
fn tcp_bind_reports_an_unresolvable_address() {
    block_on(async {
        assert!(TcpListener::bind("not an address").await.is_err());
    });
}

#[test]
fn tcp_from_std_adopts_a_sync_bound_listener() {
    let rt = Runtime::new().unwrap();
    let _guard = rt.enter();
    let std = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = std.local_addr().unwrap();
    let listener = TcpListener::from_std(std).unwrap();
    assert_eq!(listener.local_addr().unwrap(), addr);
}

#[test]
fn udp_bind_connect_and_send_to_take_tokio_address_forms() {
    block_on(async {
        let a = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let b = UdpSocket::bind(String::from("127.0.0.1:0")).await.unwrap();
        let b_addr = b.local_addr().unwrap();

        a.send_to(b"to", format!("127.0.0.1:{}", b_addr.port()))
            .await
            .unwrap();
        let mut buf = [0u8; 8];
        let (n, from) = b.recv_from(&mut buf).await.unwrap();
        assert_eq!(&buf[..n], b"to");

        b.connect(&from.to_string()).await.unwrap();
        b.send(b"back").await.unwrap();
        let (n, _) = a.recv_from(&mut buf).await.unwrap();
        assert_eq!(&buf[..n], b"back");
    });
}

#[cfg(unix)]
#[test]
fn unix_paths_take_as_ref_path_and_from_std_adopts() {
    use rusty_tokio::io::{UnixListener, UnixStream};

    let dir = std::env::temp_dir().join(format!("rt-net-parity-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("a.sock");
    let _ = std::fs::remove_file(&path);

    block_on(async {
        let listener = UnixListener::bind(&path).unwrap();
        let accept = rusty_tokio::spawn(async move { listener.accept().await.unwrap().0 });
        let client = UnixStream::connect(path.clone()).await.unwrap();
        client.write_all(b"hi").await.unwrap();
        let server = accept.await.unwrap();
        let mut buf = [0u8; 2];
        server.read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"hi");

        let other = dir.join("b.sock");
        let _ = std::fs::remove_file(&other);
        let listener =
            UnixListener::from_std(std::os::unix::net::UnixListener::bind(&other).unwrap())
                .unwrap();
        let accept = rusty_tokio::spawn(async move { listener.accept().await.unwrap().0 });
        let client =
            UnixStream::from_std(std::os::unix::net::UnixStream::connect(&other).unwrap()).unwrap();
        client.write_all(b"ok").await.unwrap();
        let server = accept.await.unwrap();
        server.read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"ok");
    });
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn builder_enable_calls_are_accepted() {
    let rt = Builder::new_multi_thread()
        .enable_all()
        .enable_io()
        .enable_time()
        .build()
        .unwrap();
    assert_eq!(rt.block_on(async { 7 }), 7);
}

#[test]
fn read_buf_has_tokios_fill_methods() {
    let mut storage = [0u8; 8];
    let mut buf = ReadBuf::new(&mut storage);
    buf.put_slice(b"abc");
    assert_eq!(buf.filled(), b"abc");
    buf.initialize_unfilled_to(2).copy_from_slice(b"de");
    buf.advance(2);
    assert_eq!(buf.filled(), b"abcde");
    assert_eq!(buf.initialize_unfilled().len(), 3);
    buf.filled_mut()[0] = b'A';
    buf.set_filled(2);
    assert_eq!(buf.filled(), b"Ab");
    buf.clear();
    assert!(buf.filled().is_empty());
    assert_eq!(buf.remaining(), 8);
}

#[test]
#[should_panic(expected = "put_slice past the end")]
fn read_buf_put_slice_past_the_end_panics() {
    let mut storage = [0u8; 2];
    ReadBuf::new(&mut storage).put_slice(b"abc");
}

#[test]
#[should_panic(expected = "initialize_unfilled_to past the end")]
fn read_buf_initialize_unfilled_to_past_the_end_panics() {
    let mut storage = [0u8; 2];
    let _ = ReadBuf::new(&mut storage).initialize_unfilled_to(3);
}

#[test]
fn cursor_reads_buffers_seeks_and_writes() {
    block_on(async {
        let mut reader = Cursor::new(b"line one\nline two\n".to_vec());
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        assert_eq!(line, "line one\n");
        reader.seek(SeekFrom::Start(5)).await.unwrap();
        let mut rest = Vec::new();
        reader.read_to_end(&mut rest).await.unwrap();
        assert_eq!(rest, b"one\nline two\n");

        let mut writer = Cursor::new(Vec::new());
        writer.write_all(b"hello").await.unwrap();
        writer.flush().await.unwrap();
        assert_eq!(writer.into_inner(), b"hello");

        let mut fixed = [0u8; 3];
        let mut slice_writer = Cursor::new(&mut fixed[..]);
        assert_eq!(slice_writer.write(b"abcdef").await.unwrap(), 3);
        assert_eq!(&fixed, b"abc");
    });
}

#[test]
fn byte_slices_and_forwarding_impls_read() {
    async fn drain<R: AsyncRead + Unpin + Send>(mut r: R) -> Vec<u8> {
        let mut out = Vec::new();
        r.read_to_end(&mut out).await.unwrap();
        out
    }
    block_on(async {
        assert_eq!(drain(&b"slice"[..]).await, b"slice");

        let mut cursor = Cursor::new(b"by ref".to_vec());
        assert_eq!(drain(&mut cursor).await, b"by ref");

        let boxed: Box<dyn AsyncRead + Unpin + Send> = Box::new(Cursor::new(b"boxed".to_vec()));
        assert_eq!(drain(boxed).await, b"boxed");
    });
}

#[test]
fn forwarding_impls_write() {
    async fn put<W: rusty_tokio::io::AsyncWrite + Unpin + Send>(mut w: W, data: &[u8]) {
        w.write_all(data).await.unwrap();
        w.flush().await.unwrap();
    }
    block_on(async {
        let mut sink = Vec::new();
        put(&mut sink, b"a").await;
        let mut boxed: Box<Vec<u8>> = Box::new(sink);
        put(&mut boxed, b"b").await;
        let dynamic: Box<dyn rusty_tokio::io::AsyncWrite + Unpin + Send> = boxed;
        put(dynamic, b"").await;
    });
}
