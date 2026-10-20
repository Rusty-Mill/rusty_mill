//! The async driver, our client against our server over an in-memory pipe,
//! plus raw-frame peers for what a well-behaved client never sends.

use std::time::Duration;

use rusty_h2::client::builder::Builder as ClientBuilder;
use rusty_h2::driver::{self, Body, ClientHandle, Error, Request, Respond};
use rusty_h2::error::ErrorCode;
use rusty_h2::frame::header::{FrameHeader, FRAME_HEADER_LEN};
use rusty_h2::frame::{DataFrame, Frame, HeadersFrame, SettingId, SettingsFrame};
use rusty_h2::hpack::{Encoder, HeaderField};
use rusty_h2::server::builder::Builder as ServerBuilder;
use rusty_h2::CONNECTION_PREFACE;
use rusty_tokio::io::{AsyncReadExt, AsyncWriteExt, TcpListener, TcpStream};
use rusty_tokio::time::timeout;

/// A connected pair of loopback sockets. (Not `duplex`: dropping one half of
/// a `DuplexStream` can deadlock, see the PR description.)
async fn pair() -> (TcpStream, TcpStream) {
    let listener = TcpListener::bind("127.0.0.1:0".parse().unwrap()).unwrap();
    let addr = listener.local_addr().unwrap();
    let (a, b) = rusty_tokio::join!(TcpStream::connect(addr), listener.accept());
    (a.unwrap(), b.unwrap().0)
}

fn hf(n: &str, v: &str) -> HeaderField {
    HeaderField::new(n, v)
}

fn request(path: &str) -> Vec<HeaderField> {
    vec![
        hf(":method", "POST"),
        hf(":scheme", "http"),
        hf(":path", path),
        hf(":authority", "localhost"),
        hf("content-type", "application/grpc"),
    ]
}

fn ok_response() -> Vec<HeaderField> {
    vec![hf(":status", "200"), hf("content-type", "application/grpc")]
}

/// Reads a body to its end: the bytes, and the trailers if there were any.
async fn drain(
    body: &mut driver::RecvStream,
) -> Result<(Vec<u8>, Option<Vec<HeaderField>>), Error> {
    let (mut bytes, mut trailers) = (Vec::new(), None);
    while let Some(piece) = body.next().await {
        match piece? {
            Body::Data(d) => bytes.extend(d),
            Body::Trailers(t) => trailers = Some(t),
        }
    }
    Ok((bytes, trailers))
}

/// A server that answers every request with its own body echoed back, then
/// `grpc-status: 0` trailers. Returns the client handle.
async fn echo_pair(client: ClientBuilder, server: ServerBuilder) -> ClientHandle {
    let (a, b) = pair().await;
    let (handle, cdrv) = driver::client(a, client);
    let (mut acceptor, sdrv) = driver::server(b, server);
    rusty_tokio::spawn(cdrv);
    rusty_tokio::spawn(sdrv);
    rusty_tokio::spawn(async move {
        while let Some(Ok((req, respond))) = acceptor.accept().await {
            rusty_tokio::spawn(echo(req, respond));
        }
    });
    handle
}

async fn echo(mut req: Request, respond: Respond) {
    let (body, _) = drain(&mut req.body).await.unwrap();
    let mut send = respond.send_response(ok_response(), false).await.unwrap();
    send.send_data(body, false).await.unwrap();
    send.send_trailers(vec![hf("grpc-status", "0")])
        .await
        .unwrap();
}

async fn call(
    h: &ClientHandle,
    path: &str,
    body: Vec<u8>,
) -> (Vec<HeaderField>, Vec<u8>, Option<Vec<HeaderField>>) {
    let (mut send, resp) = h.send_request(request(path), false).await.unwrap();
    send.send_data(body, true).await.unwrap();
    let mut resp = resp.await.unwrap();
    let (bytes, trailers) = drain(&mut resp.body).await.unwrap();
    (resp.headers, bytes, trailers)
}

// ---- the happy path ------------------------------------------------------

#[rusty_tokio::test]
async fn a_unary_call_returns_headers_body_and_trailers_in_order() {
    let h = echo_pair(ClientBuilder::new(), ServerBuilder::new()).await;
    let (headers, body, trailers) = call(&h, "/svc/Echo", b"hello".to_vec()).await;
    assert_eq!(headers, ok_response());
    assert_eq!(body, b"hello");
    assert_eq!(trailers, Some(vec![hf("grpc-status", "0")]));
}

#[rusty_tokio::test]
async fn a_request_without_a_body_is_answered() {
    let h = echo_pair(ClientBuilder::new(), ServerBuilder::new()).await;
    let (_send, resp) = h.send_request(request("/svc/Empty"), true).await.unwrap();
    let mut resp = resp.await.unwrap();
    let (body, trailers) = drain(&mut resp.body).await.unwrap();
    assert!(body.is_empty());
    assert!(trailers.is_some());
}

#[rusty_tokio::test]
async fn many_concurrent_calls_do_not_mix_up_their_bodies() {
    let h = echo_pair(ClientBuilder::new(), ServerBuilder::new()).await;
    let mut tasks = Vec::new();
    for i in 0..64u8 {
        let h = h.clone();
        tasks.push(rusty_tokio::spawn(async move {
            let body = vec![i; 1 + usize::from(i) * 100];
            let (_, got, _) = call(&h, "/svc/Echo", body.clone()).await;
            assert_eq!(got, body);
        }));
    }
    for t in tasks {
        timeout(Duration::from_secs(10), t).await.unwrap().unwrap();
    }
}

#[rusty_tokio::test]
async fn a_body_far_larger_than_the_window_flows_both_ways() {
    let h = echo_pair(ClientBuilder::new(), ServerBuilder::new()).await;
    let body: Vec<u8> = (0..3_000_000u32).map(|i| (i % 251) as u8).collect();
    let (_, got, _) = timeout(Duration::from_secs(20), call(&h, "/svc/Echo", body.clone()))
        .await
        .expect("a large transfer must keep making progress");
    assert!(got == body);
}

#[rusty_tokio::test]
async fn ping_is_acknowledged() {
    let h = echo_pair(ClientBuilder::new(), ServerBuilder::new()).await;
    timeout(Duration::from_secs(5), h.ping())
        .await
        .unwrap()
        .unwrap();
}

#[rusty_tokio::test]
async fn requests_beyond_the_peers_stream_limit_wait_their_turn() {
    let mut sb = ServerBuilder::new();
    sb.max_concurrent_streams(2);
    let h = echo_pair(ClientBuilder::new(), sb).await;
    let mut tasks = Vec::new();
    for i in 0..10u8 {
        let h = h.clone();
        tasks.push(rusty_tokio::spawn(async move {
            let (_, got, _) = call(&h, "/svc/Echo", vec![i; 10]).await;
            assert_eq!(got, vec![i; 10]);
        }));
    }
    for t in tasks {
        timeout(Duration::from_secs(10), t).await.unwrap().unwrap();
    }
}

// ---- backpressure ---------------------------------------------------------

#[rusty_tokio::test]
async fn a_receiver_that_does_not_read_stalls_the_sender() {
    let (a, b) = pair().await;
    let (h, cdrv) = driver::client(a, ClientBuilder::new());
    let (mut acceptor, sdrv) = driver::server(b, ServerBuilder::new());
    rusty_tokio::spawn(cdrv);
    rusty_tokio::spawn(sdrv);
    let (mut send, _resp) = h.send_request(request("/svc/Slow"), false).await.unwrap();
    let (mut req, _respond) = acceptor.accept().await.unwrap().unwrap();

    // 1 MiB cannot fit the default 64 KiB windows while nobody reads.
    let mut sending =
        rusty_tokio::spawn(async move { send.send_data(vec![0; 1 << 20], true).await });
    assert!(
        timeout(Duration::from_millis(300), &mut sending)
            .await
            .is_err(),
        "the sender must wait for window updates"
    );
    // Reading releases capacity and lets it finish.
    let reader = rusty_tokio::spawn(async move { drain(&mut req.body).await.unwrap().0.len() });
    timeout(Duration::from_secs(10), sending)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(reader.await.unwrap(), 1 << 20);
}

// ---- resets, shutdown, close ----------------------------------------------

#[rusty_tokio::test]
async fn a_client_reset_reaches_the_server_body() {
    let (a, b) = pair().await;
    let (h, cdrv) = driver::client(a, ClientBuilder::new());
    let (mut acceptor, sdrv) = driver::server(b, ServerBuilder::new());
    rusty_tokio::spawn(cdrv);
    rusty_tokio::spawn(sdrv);
    let (mut send, _resp) = h.send_request(request("/svc/Cancel"), false).await.unwrap();
    send.send_data(b"part".to_vec(), false).await.unwrap();
    let (mut req, _respond) = acceptor.accept().await.unwrap().unwrap();
    send.reset(ErrorCode::Cancel);
    let mut saw_reset = false;
    while let Some(piece) = req.body.next().await {
        if matches!(piece, Err(Error::Reset(ErrorCode::Cancel))) {
            saw_reset = true;
        }
    }
    assert!(saw_reset);
}

#[rusty_tokio::test]
async fn a_server_reset_fails_the_response() {
    let (a, b) = pair().await;
    let (h, cdrv) = driver::client(a, ClientBuilder::new());
    let (mut acceptor, sdrv) = driver::server(b, ServerBuilder::new());
    rusty_tokio::spawn(cdrv);
    rusty_tokio::spawn(sdrv);
    let (_send, resp) = h.send_request(request("/svc/Refuse"), true).await.unwrap();
    let (_req, respond) = acceptor.accept().await.unwrap().unwrap();
    let mut send = respond.send_response(ok_response(), false).await.unwrap();
    send.reset(ErrorCode::RefusedStream);
    match resp.await {
        Err(Error::Reset(ErrorCode::RefusedStream)) => {}
        Ok(mut r) => assert!(matches!(
            r.body.next().await,
            Some(Err(Error::Reset(ErrorCode::RefusedStream)))
        )),
        other => panic!("{:?}", other.err()),
    }
}

#[rusty_tokio::test]
async fn graceful_shutdown_finishes_in_flight_calls_and_refuses_new_ones() {
    let (a, b) = pair().await;
    let (h, cdrv) = driver::client(a, ClientBuilder::new());
    let (mut acceptor, sdrv) = driver::server(b, ServerBuilder::new());
    let client_task = rusty_tokio::spawn(cdrv);
    let server_task = rusty_tokio::spawn(sdrv);
    let (mut send, resp) = h.send_request(request("/svc/Echo"), false).await.unwrap();
    let (req, respond) = acceptor.accept().await.unwrap().unwrap();
    acceptor.graceful_shutdown();

    send.send_data(b"late".to_vec(), true).await.unwrap();
    rusty_tokio::spawn(echo(req, respond));
    let mut resp = resp.await.unwrap();
    assert_eq!(drain(&mut resp.body).await.unwrap().0, b"late");

    // A new request after GOAWAY is refused, not hung.
    let again = timeout(Duration::from_secs(5), async {
        let (_s, r) = h.send_request(request("/svc/Echo"), true).await?;
        r.await.map(|_| ())
    })
    .await
    .unwrap();
    assert!(again.is_err());
    assert!(timeout(Duration::from_secs(5), client_task)
        .await
        .unwrap()
        .unwrap()
        .is_ok());
    assert!(timeout(Duration::from_secs(5), server_task)
        .await
        .unwrap()
        .unwrap()
        .is_ok());
}

#[rusty_tokio::test]
async fn dropping_every_client_handle_closes_the_connection_cleanly() {
    let (a, b) = pair().await;
    let (h, cdrv) = driver::client(a, ClientBuilder::new());
    let (mut acceptor, sdrv) = driver::server(b, ServerBuilder::new());
    let client_task = rusty_tokio::spawn(cdrv);
    let server_task = rusty_tokio::spawn(sdrv);
    drop(h);
    assert!(timeout(Duration::from_secs(5), acceptor.accept())
        .await
        .unwrap()
        .is_none());
    assert!(timeout(Duration::from_secs(5), client_task)
        .await
        .unwrap()
        .unwrap()
        .is_ok());
    let server_result = timeout(Duration::from_secs(5), server_task)
        .await
        .unwrap()
        .unwrap();
    assert!(server_result.is_ok(), "{server_result:?}");
}

#[rusty_tokio::test]
async fn a_transport_that_dies_fails_pending_calls() {
    let (a, b) = pair().await;
    let (h, cdrv) = driver::client(a, ClientBuilder::new());
    rusty_tokio::spawn(cdrv);
    // A peer that says hello, takes the request, then vanishes.
    b.write_all(&wire(&Frame::Settings(SettingsFrame {
        ack: false,
        settings: vec![],
    })))
    .await
    .unwrap();
    let (_send, resp) = h.send_request(request("/svc/Echo"), true).await.unwrap();
    drop(b);
    assert!(timeout(Duration::from_secs(5), resp)
        .await
        .unwrap()
        .is_err());
}

// ---- a raw, hostile peer ---------------------------------------------------

fn wire(frame: &Frame) -> Vec<u8> {
    let mut out = Vec::new();
    frame.encode(&mut out);
    out
}

/// Reads one frame off a raw pipe, or `None` at end of input.
async fn read_frame<R: AsyncReadExt + Unpin + Send>(r: &mut R) -> Option<Frame> {
    let mut head = [0u8; FRAME_HEADER_LEN];
    r.read_exact(&mut head).await.ok()?;
    let header = FrameHeader::decode(&head).unwrap();
    let mut payload = vec![0u8; header.length as usize];
    r.read_exact(&mut payload).await.ok()?;
    Some(Frame::decode(&header, &payload).unwrap())
}

/// A raw client that has said hello to a driven server.
async fn raw_client() -> (TcpStream, driver::Acceptor) {
    let (raw, acceptor, _driver) = raw_client_with_driver().await;
    (raw, acceptor)
}

/// Like [`raw_client`], also returning the server's driver task.
async fn raw_client_with_driver() -> (
    TcpStream,
    driver::Acceptor,
    rusty_tokio::JoinHandle<Result<(), Error>>,
) {
    let (raw, b) = pair().await;
    let mut small = ServerBuilder::new();
    small.initial_window_size(10_000);
    let (acceptor, sdrv) = driver::server(b, small);
    let task = rusty_tokio::spawn(sdrv);
    raw.write_all(CONNECTION_PREFACE).await.unwrap();
    raw.write_all(&wire(&Frame::Settings(SettingsFrame {
        ack: false,
        settings: vec![],
    })))
    .await
    .unwrap();
    (raw, acceptor, task)
}

fn raw_headers(id: u32, end_stream: bool) -> Frame {
    let mut block = Vec::new();
    Encoder::default().encode(&request("/svc/Raw"), &mut block);
    Frame::Headers(HeadersFrame {
        stream_id: id,
        end_stream,
        end_headers: true,
        priority: None,
        header_block_fragment: block,
    })
}

#[rusty_tokio::test]
async fn data_beyond_the_advertised_window_resets_only_that_stream() {
    let (mut raw, _acceptor) = raw_client().await;
    raw.write_all(&wire(&raw_headers(1, false))).await.unwrap();
    for _ in 0..5 {
        // 5 x 16,000 = 80,000 > 65,535, never read by the application.
        raw.write_all(&wire(&Frame::Data(DataFrame::new(
            1,
            vec![0; 16_000],
            false,
        ))))
        .await
        .unwrap();
    }
    let reset = timeout(Duration::from_secs(5), async {
        while let Some(f) = read_frame(&mut raw).await {
            if let Frame::RstStream(r) = f {
                return Some(r);
            }
            if matches!(f, Frame::GoAway(_)) {
                return None;
            }
        }
        None
    })
    .await
    .unwrap()
    .expect("a stream error, not a connection error");
    assert_eq!(
        (reset.stream_id, reset.error_code),
        (1, ErrorCode::FlowControlError)
    );
}

#[rusty_tokio::test]
async fn garbage_instead_of_the_preface_is_refused_and_closed() {
    let (raw, b) = pair().await;
    let (_acceptor, sdrv) = driver::server(b, ServerBuilder::new());
    let task = rusty_tokio::spawn(sdrv);
    raw.write_all(b"GET / HTTP/1.1\r\nHost: x\r\n\r\n")
        .await
        .unwrap();
    let result = timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(result, Err(Error::Protocol(_))));
}

#[rusty_tokio::test]
async fn a_protocol_violation_sends_goaway_and_ends_the_driver() {
    let (mut raw, b) = pair().await;
    let (_acceptor, sdrv) = driver::server(b, ServerBuilder::new());
    let task = rusty_tokio::spawn(sdrv);
    raw.write_all(CONNECTION_PREFACE).await.unwrap();
    // DATA on stream 0 is a connection error.
    raw.write_all(&wire(&Frame::Settings(SettingsFrame {
        ack: false,
        settings: vec![],
    })))
    .await
    .unwrap();
    raw.write_all(&wire(&Frame::Data(DataFrame::new(0, vec![1], false))))
        .await
        .unwrap();
    let goaway = timeout(Duration::from_secs(5), async {
        while let Some(f) = read_frame(&mut raw).await {
            if let Frame::GoAway(g) = f {
                return Some(g);
            }
        }
        None
    })
    .await
    .unwrap()
    .expect("GOAWAY before closing");
    assert_eq!(goaway.error_code, ErrorCode::ProtocolError);
    assert!(timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap()
        .is_err());
}

#[rusty_tokio::test]
async fn a_flood_of_pings_stalls_the_peer_instead_of_growing_memory() {
    let (raw, _acceptor, driver_task) = raw_client_with_driver().await;
    let ping = wire(&Frame::Ping(rusty_h2::frame::PingFrame {
        ack: false,
        opaque_data: [7; 8],
    }));
    let flood = ping.repeat(4_000_000); // 68 MB of pings; more acks than any socket buffers hold
    let (mut rd, mut wr) = raw.into_split();
    let mut writer = rusty_tokio::spawn(async move {
        let result = wr.write_all(&flood).await;
        (result, wr)
    });
    let early = timeout(Duration::from_secs(2), &mut writer).await;
    assert!(
        early.is_err(),
        "the driver must stop reading while its acks are unread; the writer ended with {:?}, the driver finished: {}",
        early.map(|r| r.map(|(res, _)| res)),
        driver_task.is_finished()
    );
    // Reading the acks lets the flood through.
    let drain = rusty_tokio::spawn(async move {
        let mut buf = vec![0u8; 64 * 1024];
        while rd.read(&mut buf).await.is_ok_and(|n| n > 0) {}
    });
    let (result, _wr) = timeout(Duration::from_secs(20), writer)
        .await
        .unwrap()
        .unwrap();
    result.unwrap();
    drain.abort();
}

#[rusty_tokio::test]
async fn streams_the_server_never_processed_fail_with_goaway_and_earlier_ones_finish() {
    let (a, raw) = pair().await;
    let (h, cdrv) = driver::client(a, ClientBuilder::new());
    rusty_tokio::spawn(cdrv);
    raw.write_all(&wire(&Frame::Settings(SettingsFrame {
        ack: false,
        settings: vec![],
    })))
    .await
    .unwrap();
    let (_s1, r1) = h.send_request(request("/svc/One"), true).await.unwrap();
    let (_s3, r3) = h.send_request(request("/svc/Three"), true).await.unwrap();
    // The server processed stream 1 only.
    raw.write_all(&wire(&Frame::GoAway(rusty_h2::frame::GoAwayFrame {
        last_stream_id: 1,
        error_code: ErrorCode::NoError,
        debug_data: vec![],
    })))
    .await
    .unwrap();
    assert!(matches!(
        timeout(Duration::from_secs(5), r3).await.unwrap(),
        Err(Error::GoAway(ErrorCode::NoError))
    ));
    // Stream 1 still gets its answer.
    let mut enc = Encoder::default();
    let mut block = Vec::new();
    enc.encode(&[hf(":status", "200")], &mut block);
    raw.write_all(&wire(&Frame::Headers(HeadersFrame {
        stream_id: 1,
        end_stream: true,
        end_headers: true,
        priority: None,
        header_block_fragment: block,
    })))
    .await
    .unwrap();
    assert!(timeout(Duration::from_secs(5), r1).await.unwrap().is_ok());
}

#[rusty_tokio::test]
async fn a_server_in_graceful_shutdown_refuses_streams_it_did_not_see_coming() {
    let (mut raw, mut acceptor) = raw_client().await;
    raw.write_all(&wire(&raw_headers(1, true))).await.unwrap();
    let (_req, _respond) = acceptor.accept().await.unwrap().unwrap();
    acceptor.graceful_shutdown();
    // The client had not yet seen the GOAWAY when it started stream 3.
    let mut enc = Encoder::default();
    let mut block = Vec::new();
    enc.encode(&request("/svc/Late"), &mut block);
    raw.write_all(&wire(&Frame::Headers(HeadersFrame {
        stream_id: 3,
        end_stream: true,
        end_headers: true,
        priority: None,
        header_block_fragment: block,
    })))
    .await
    .unwrap();
    let reset = timeout(Duration::from_secs(5), async {
        while let Some(f) = read_frame(&mut raw).await {
            if let Frame::RstStream(r) = f {
                return Some(r);
            }
        }
        None
    })
    .await
    .unwrap()
    .expect("RST_STREAM for the late stream");
    assert_eq!(
        (reset.stream_id, reset.error_code),
        (3, ErrorCode::RefusedStream)
    );
}

// ---- review round: lifecycle, flushing and wake-ups ---------------------

/// Reads frames off a raw socket until `pick` returns something.
async fn read_until<T>(raw: &mut TcpStream, mut pick: impl FnMut(Frame) -> Option<T>) -> Option<T> {
    timeout(Duration::from_secs(5), async {
        while let Some(f) = read_frame(raw).await {
            if let Some(t) = pick(f) {
                return Some(t);
            }
        }
        None
    })
    .await
    .unwrap_or(None)
}

fn raw_response(id: u32, end_stream: bool) -> Frame {
    let mut block = Vec::new();
    Encoder::default().encode(&[hf(":status", "200")], &mut block);
    Frame::Headers(HeadersFrame {
        stream_id: id,
        end_stream,
        end_headers: true,
        priority: None,
        header_block_fragment: block,
    })
}

#[rusty_tokio::test]
async fn a_default_servers_opening_settings_never_enable_push() {
    let (a, mut raw) = pair().await;
    let (_acceptor, sdrv) = driver::server(a, ServerBuilder::new());
    rusty_tokio::spawn(sdrv);
    raw.write_all(CONNECTION_PREFACE).await.unwrap();
    raw.write_all(&wire(&Frame::Settings(SettingsFrame {
        ack: false,
        settings: vec![],
    })))
    .await
    .unwrap();
    let settings = read_until(&mut raw, |f| match f {
        Frame::Settings(s) if !s.ack => Some(s),
        _ => None,
    })
    .await
    .expect("the server's SETTINGS");
    // RFC 9113 §6.5.2: a server must not send ENABLE_PUSH = 1.
    assert!(
        settings
            .settings
            .iter()
            .all(|x| x.id != SettingId::EnablePush || x.value == 0),
        "{settings:?}"
    );
}

#[rusty_tokio::test]
async fn an_empty_end_stream_wakes_a_pending_request_body_read() {
    let (raw, mut acceptor) = raw_client().await;
    raw.write_all(&wire(&raw_headers(1, false))).await.unwrap();
    let (mut req, _respond) = acceptor.accept().await.unwrap().unwrap();
    let reader = rusty_tokio::spawn(async move { req.body.next().await.is_none() });
    rusty_tokio::time::sleep(Duration::from_millis(150)).await; // the read is pending
    raw.write_all(&wire(&Frame::Data(DataFrame::new(1, Vec::new(), true))))
        .await
        .unwrap();
    assert!(timeout(Duration::from_secs(5), reader)
        .await
        .unwrap()
        .unwrap());
}

#[rusty_tokio::test]
async fn an_empty_end_stream_wakes_a_pending_response_body_read() {
    let (a, raw) = pair().await;
    let (h, cdrv) = driver::client(a, ClientBuilder::new());
    rusty_tokio::spawn(cdrv);
    raw.write_all(&wire(&Frame::Settings(SettingsFrame {
        ack: false,
        settings: vec![],
    })))
    .await
    .unwrap();
    let (_s, resp) = h.send_request(request("/svc/Empty"), true).await.unwrap();
    raw.write_all(&wire(&raw_response(1, false))).await.unwrap();
    let mut resp = timeout(Duration::from_secs(5), resp)
        .await
        .unwrap()
        .unwrap();
    let reader = rusty_tokio::spawn(async move { resp.body.next().await.is_none() });
    rusty_tokio::time::sleep(Duration::from_millis(150)).await;
    raw.write_all(&wire(&Frame::Data(DataFrame::new(1, Vec::new(), true))))
        .await
        .unwrap();
    assert!(timeout(Duration::from_secs(5), reader)
        .await
        .unwrap()
        .unwrap());
}

#[rusty_tokio::test]
async fn a_driver_dropped_before_its_first_poll_closes_the_handles() {
    let (a, b) = pair().await;
    let (h, cdrv) = driver::client(a, ClientBuilder::new());
    let (mut acceptor, sdrv) = driver::server(b, ServerBuilder::new());
    drop(cdrv);
    drop(sdrv);
    let call = timeout(
        Duration::from_secs(5),
        h.send_request(request("/svc/Echo"), true),
    )
    .await
    .expect("a closed connection must not leave the request waiting");
    assert!(matches!(call, Err(Error::Closed)));
    let ping = timeout(Duration::from_secs(5), h.ping()).await.unwrap();
    assert!(matches!(ping, Err(Error::Closed)));
    let accepted = timeout(Duration::from_secs(5), acceptor.accept())
        .await
        .unwrap();
    assert!(accepted.is_none());
}

#[rusty_tokio::test]
async fn an_aborted_driver_fails_every_pending_operation() {
    let (a, b) = pair().await;
    let (h, cdrv) = driver::client(a, ClientBuilder::new());
    let (mut acceptor, sdrv) = driver::server(b, ServerBuilder::new());
    let client_task = rusty_tokio::spawn(cdrv);
    let server_task = rusty_tokio::spawn(sdrv);
    let (mut send, resp) = h.send_request(request("/svc/Hang"), false).await.unwrap();
    send.send_data(b"x".to_vec(), false).await.unwrap();
    let (mut req, _respond) = acceptor.accept().await.unwrap().unwrap();
    assert!(matches!(req.body.next().await, Some(Ok(Body::Data(_)))));

    let pending_resp = rusty_tokio::spawn(resp);
    let pending_body = rusty_tokio::spawn(async move { req.body.next().await });
    let pending_accept = rusty_tokio::spawn(async move { acceptor.accept().await.is_none() });
    rusty_tokio::time::sleep(Duration::from_millis(150)).await;

    client_task.abort();
    server_task.abort();
    let five = Duration::from_secs(5);
    assert!(timeout(five, pending_resp).await.unwrap().unwrap().is_err());
    assert!(matches!(
        timeout(five, pending_body).await.unwrap().unwrap(),
        Some(Err(_)) | None
    ));

    // A ping nobody answers: the peer is a raw socket that never replies.
    let (c, quiet) = pair().await;
    let (hp, pdrv) = driver::client(c, ClientBuilder::new());
    let ping_task = rusty_tokio::spawn(pdrv);
    let pending_ping = rusty_tokio::spawn(async move { hp.ping().await });
    rusty_tokio::time::sleep(Duration::from_millis(150)).await;
    ping_task.abort();
    let ping = timeout(five, pending_ping).await.unwrap().unwrap();
    assert!(ping.is_err());
    drop(quiet);
    assert!(timeout(five, pending_accept).await.unwrap().unwrap());
}

/// Waits until the server has applied every frame sent so far: a PING is
/// answered in order, so its ACK means the earlier frames are done. Returns the
/// connection-level credit (WINDOW_UPDATE on stream 0) seen on the way.
async fn barrier(raw: &mut TcpStream) -> u32 {
    let marker = [9u8; 8];
    raw.write_all(&wire(&Frame::Ping(rusty_h2::frame::PingFrame {
        ack: false,
        opaque_data: marker,
    })))
    .await
    .unwrap();
    let mut credit = 0;
    read_until(raw, |f| match f {
        Frame::WindowUpdate(w) if w.stream_id == 0 => {
            credit += w.window_size_increment;
            None
        }
        Frame::Ping(p) if p.ack && p.opaque_data == marker => Some(()),
        _ => None,
    })
    .await
    .expect("the PING barrier is acknowledged");
    credit
}

fn raw_rst(id: u32) -> Frame {
    Frame::RstStream(rusty_h2::frame::RstStreamFrame {
        stream_id: id,
        error_code: ErrorCode::Cancel,
    })
}

async fn reset_before_accept(split: bool) {
    let (mut raw, mut acceptor) = raw_client().await;
    let mut burst = wire(&raw_headers(1, false));
    burst.extend(wire(&raw_rst(1)));
    burst.extend(wire(&raw_headers(3, true)));
    if split {
        // However the bytes are cut up, the result is the same.
        for byte in &burst {
            raw.write_all(&[*byte]).await.unwrap();
        }
    } else {
        raw.write_all(&burst).await.unwrap();
    }
    // Accept only once the reset has been applied.
    barrier(&mut raw).await;
    let (req, _respond) = timeout(Duration::from_secs(5), acceptor.accept())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(
        req.headers
            .iter()
            .find(|h| h.name == b":path")
            .map(|h| h.value.clone()),
        Some(b"/svc/Raw".to_vec())
    );
    assert!(
        timeout(Duration::from_millis(300), acceptor.accept())
            .await
            .is_err(),
        "the reset stream must not be offered later"
    );
}

#[rusty_tokio::test]
async fn a_stream_reset_before_it_is_accepted_is_reclaimed() {
    reset_before_accept(false).await;
}

#[rusty_tokio::test]
async fn a_reset_stream_is_reclaimed_however_the_bytes_are_split() {
    reset_before_accept(true).await;
}

#[rusty_tokio::test]
async fn reclaiming_an_incomplete_unaccepted_request_returns_its_connection_credit_once() {
    let (mut raw, _acceptor) = raw_client().await;
    let mut burst = wire(&raw_headers(1, false));
    burst.extend(wire(&Frame::Data(DataFrame::new(1, vec![0; 5_000], false))));
    burst.extend(wire(&raw_rst(1)));
    raw.write_all(&burst).await.unwrap();
    assert_eq!(barrier(&mut raw).await, 5_000);
    assert_eq!(barrier(&mut raw).await, 0, "and nothing more later");
}

#[rusty_tokio::test]
async fn reclaiming_a_complete_unaccepted_request_returns_its_connection_credit_once() {
    let (mut raw, _acceptor) = raw_client().await;
    // The whole request arrives, then is reset before anyone accepts it.
    let mut burst = wire(&raw_headers(1, false));
    burst.extend(wire(&Frame::Data(DataFrame::new(1, vec![0; 5_000], true))));
    burst.extend(wire(&raw_rst(1)));
    raw.write_all(&burst).await.unwrap();
    assert_eq!(barrier(&mut raw).await, 5_000);
    assert_eq!(barrier(&mut raw).await, 0, "and nothing more later");
}

#[rusty_tokio::test]
async fn refusing_unaccepted_requests_on_acceptor_drop_returns_their_credit_once() {
    let (mut raw, acceptor) = raw_client().await;
    let mut burst = wire(&raw_headers(1, false));
    burst.extend(wire(&Frame::Data(DataFrame::new(1, vec![0; 5_000], true))));
    raw.write_all(&burst).await.unwrap();
    assert_eq!(
        barrier(&mut raw).await,
        0,
        "nothing is returned while it waits"
    );
    drop(acceptor);
    // The connection now winds down, so read it to its end rather than ping it.
    let mut credit = 0;
    timeout(Duration::from_secs(5), async {
        while let Some(f) = read_frame(&mut raw).await {
            if let Frame::WindowUpdate(w) = f {
                if w.stream_id == 0 {
                    credit += w.window_size_increment;
                }
            }
        }
    })
    .await
    .expect("the connection ends");
    assert_eq!(credit, 5_000, "returned once, in full");
}

#[rusty_tokio::test]
async fn unaccepted_requests_are_bounded() {
    let (mut raw, mut acceptor) = raw_client().await;
    let mut written = 0u32;
    for i in 0..600u32 {
        raw.write_all(&wire(&raw_headers(2 * i + 1, false)))
            .await
            .unwrap();
        written += 1;
    }
    assert_eq!(written, 600);
    let refused = read_until(&mut raw, |f| match f {
        Frame::RstStream(r) if r.error_code == ErrorCode::RefusedStream => Some(r.stream_id),
        _ => None,
    })
    .await;
    assert!(
        refused.is_some(),
        "requests beyond the pending limit are refused"
    );
    let mut accepted = 0;
    while let Ok(Some(Ok(_))) = timeout(Duration::from_millis(200), acceptor.accept()).await {
        accepted += 1;
    }
    assert!(accepted > 0 && accepted < 600, "accepted {accepted}");
}

#[rusty_tokio::test]
async fn dropping_the_acceptor_resets_unaccepted_requests_and_the_driver_finishes() {
    let (mut raw, acceptor, task) = raw_client_with_driver().await;
    raw.write_all(&wire(&raw_headers(1, false))).await.unwrap();
    raw.write_all(&wire(&raw_headers(3, true))).await.unwrap();
    rusty_tokio::time::sleep(Duration::from_millis(150)).await;
    drop(acceptor);
    let mut reset = std::collections::BTreeSet::new();
    read_until(&mut raw, |f| {
        if let Frame::RstStream(r) = f {
            reset.insert(r.stream_id);
        }
        (reset.len() == 2).then_some(())
    })
    .await
    .expect("both unaccepted requests are reset");
    assert!(timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap()
        .is_ok());
}

#[rusty_tokio::test]
async fn a_response_that_cannot_be_encoded_resets_the_stream() {
    let (mut raw, mut acceptor, task) = raw_client_with_driver().await;
    raw.write_all(&wire(&raw_headers(1, true))).await.unwrap();
    let (_req, respond) = acceptor.accept().await.unwrap().unwrap();
    // Larger than the peer's maximum frame size: there is no CONTINUATION on send.
    let huge = vec![hf(":status", "200"), hf("x-big", &"v".repeat(40_000))];
    assert!(respond.send_response(huge, true).await.is_err());
    let rst = read_until(&mut raw, |f| match f {
        Frame::RstStream(r) => Some(r),
        _ => None,
    })
    .await
    .expect("the peer must be told");
    assert_eq!(
        (rst.stream_id, rst.error_code),
        (1, ErrorCode::InternalError)
    );
    acceptor.graceful_shutdown();
    assert!(timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap()
        .is_ok());
}

/// Switches a test flips on a [`Buffered`] transport from outside.
#[derive(Clone, Default)]
struct Knobs {
    shut_down: std::sync::Arc<std::sync::atomic::AtomicBool>,
    fail_flush: std::sync::Arc<std::sync::atomic::AtomicBool>,
    fail_write: std::sync::Arc<std::sync::atomic::AtomicBool>,
    flush_pending_once: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

/// A transport that buffers writes until flushed and can misbehave.
struct Buffered {
    inner: TcpStream,
    buf: Vec<u8>,
    shutdown_pending_once: bool,
    shutdown_error: bool,
    knobs: Knobs,
}

impl Buffered {
    fn new(inner: TcpStream) -> Buffered {
        Buffered {
            inner,
            buf: Vec::new(),
            shutdown_pending_once: false,
            shutdown_error: false,
            knobs: Knobs::default(),
        }
    }

    fn drain(&mut self, cx: &mut std::task::Context<'_>) -> std::task::Poll<std::io::Result<()>> {
        use rusty_tokio::io::AsyncWrite;
        while !self.buf.is_empty() {
            match std::pin::Pin::new(&mut self.inner).poll_write(cx, &self.buf) {
                std::task::Poll::Ready(Ok(n)) => {
                    self.buf.drain(..n);
                }
                other => return other.map(|r| r.map(|_| ())),
            }
        }
        std::pin::Pin::new(&mut self.inner).poll_flush(cx)
    }
}

impl rusty_tokio::io::AsyncRead for Buffered {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut rusty_tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl rusty_tokio::io::AsyncWrite for Buffered {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
        data: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        if self
            .knobs
            .fail_write
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            return std::task::Poll::Ready(Err(std::io::Error::other("write failed")));
        }
        self.buf.extend_from_slice(data);
        std::task::Poll::Ready(Ok(data.len()))
    }

    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        use std::sync::atomic::Ordering::SeqCst;
        if self.knobs.fail_flush.load(SeqCst) {
            return std::task::Poll::Ready(Err(std::io::Error::other("flush failed")));
        }
        if self.knobs.flush_pending_once.swap(false, SeqCst) {
            cx.waker().wake_by_ref();
            return std::task::Poll::Pending;
        }
        self.drain(cx)
    }

    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        if self.shutdown_pending_once {
            self.shutdown_pending_once = false;
            cx.waker().wake_by_ref();
            return std::task::Poll::Pending;
        }
        if self.shutdown_error {
            return std::task::Poll::Ready(Err(std::io::Error::other("shutdown failed")));
        }
        match self.drain(cx) {
            std::task::Poll::Ready(Ok(())) => {}
            other => return other,
        }
        let done = std::pin::Pin::new(&mut self.inner).poll_shutdown(cx);
        if done.is_ready() {
            self.knobs
                .shut_down
                .store(true, std::sync::atomic::Ordering::SeqCst);
        }
        done
    }
}

#[rusty_tokio::test]
async fn the_driver_flushes_a_buffered_transport() {
    let (a, mut raw) = pair().await;
    let (_acceptor, sdrv) = driver::server(Buffered::new(a), ServerBuilder::new());
    rusty_tokio::spawn(sdrv);
    raw.write_all(CONNECTION_PREFACE).await.unwrap();
    raw.write_all(&wire(&Frame::Settings(SettingsFrame {
        ack: false,
        settings: vec![],
    })))
    .await
    .unwrap();
    // Without a flush the server's SETTINGS would sit in the transport's buffer.
    let got = read_until(&mut raw, |f| {
        matches!(f, Frame::Settings(s) if !s.ack).then_some(())
    })
    .await;
    assert!(got.is_some(), "the handshake bytes must reach the peer");
}

#[rusty_tokio::test]
async fn the_driver_waits_for_shutdown_to_complete() {
    let (a, b) = pair().await;
    let mut transport = Buffered::new(a);
    transport.shutdown_pending_once = true;
    let shut_down = transport.knobs.shut_down.clone();
    let (h, cdrv) = driver::client(transport, ClientBuilder::new());
    let (_acceptor, sdrv) = driver::server(b, ServerBuilder::new());
    rusty_tokio::spawn(sdrv);
    let task = rusty_tokio::spawn(cdrv);
    drop(h);
    assert!(timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap()
        .is_ok());
    assert!(
        shut_down.load(std::sync::atomic::Ordering::SeqCst),
        "the driver must not finish before the transport's shutdown completes"
    );
}

#[rusty_tokio::test]
async fn a_failed_shutdown_is_reported() {
    let (a, b) = pair().await;
    let mut transport = Buffered::new(a);
    transport.shutdown_error = true;
    let (h, cdrv) = driver::client(transport, ClientBuilder::new());
    let (_acceptor, sdrv) = driver::server(b, ServerBuilder::new());
    rusty_tokio::spawn(sdrv);
    let task = rusty_tokio::spawn(cdrv);
    drop(h);
    let result = timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(result, Err(Error::Io(_))), "{result:?}");
}

/// A client over a buffered transport whose handshake has finished, and the
/// task running its driver. The peer is a normal server.
async fn buffered_client() -> (
    ClientHandle,
    rusty_tokio::JoinHandle<Result<(), Error>>,
    Knobs,
) {
    let (a, b) = pair().await;
    let transport = Buffered::new(a);
    let knobs = transport.knobs.clone();
    let (h, cdrv) = driver::client(transport, ClientBuilder::new());
    let (acceptor, sdrv) = driver::server(b, ServerBuilder::new());
    rusty_tokio::spawn(sdrv);
    std::mem::forget(acceptor); // keep the server side alive without a handle
    let task = rusty_tokio::spawn(cdrv);
    timeout(Duration::from_secs(5), h.ping())
        .await
        .unwrap()
        .unwrap();
    (h, task, knobs)
}

#[rusty_tokio::test]
async fn a_final_flush_error_turns_a_clean_finish_into_a_failure() {
    let (h, task, knobs) = buffered_client().await;
    knobs
        .fail_flush
        .store(true, std::sync::atomic::Ordering::SeqCst);
    drop(h); // the connection winds down cleanly; its last bytes cannot be flushed
    let result = timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(result, Err(Error::Io(_))), "{result:?}");
}

#[rusty_tokio::test]
async fn a_final_write_error_turns_a_clean_finish_into_a_failure() {
    let (h, task, knobs) = buffered_client().await;
    knobs
        .fail_write
        .store(true, std::sync::atomic::Ordering::SeqCst);
    drop(h);
    let result = timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(result, Err(Error::Io(_))), "{result:?}");
}

#[rusty_tokio::test]
async fn a_pending_final_flush_is_resumed_and_the_finish_stays_clean() {
    let (h, task, knobs) = buffered_client().await;
    knobs
        .flush_pending_once
        .store(true, std::sync::atomic::Ordering::SeqCst);
    drop(h);
    let result = timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap();
    assert!(result.is_ok(), "{result:?}");
}
