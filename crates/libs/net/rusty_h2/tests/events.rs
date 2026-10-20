//! What a connection delivers to the application, and what it sends: header
//! blocks and trailers, body bytes, resets, and the flow-control windows.

use rusty_h2::connect::{Connection, Inbound, PeerType, ServerSettings};
use rusty_h2::error::{ErrorCode, H2Error};
use rusty_h2::frame::{
    DataFrame, Frame, GoAwayFrame, HeadersFrame, RstStreamFrame, Setting, SettingId, SettingsFrame,
    WindowUpdateFrame,
};
use rusty_h2::hpack::{Decoder, Encoder, HeaderField};

fn hf(name: &str, value: &str) -> HeaderField {
    HeaderField::new(name, value)
}

fn request() -> Vec<HeaderField> {
    vec![
        hf(":method", "POST"),
        hf(":scheme", "http"),
        hf(":path", "/svc/Method"),
        hf(":authority", "localhost"),
        hf("content-type", "application/grpc"),
    ]
}

fn server() -> Connection {
    let mut c = Connection::new(ServerSettings::default(), PeerType::Server);
    c.mark_preface_seen();
    c
}

fn client() -> Connection {
    Connection::new(ServerSettings::default(), PeerType::Client)
}

/// A peer's HEADERS frame, encoded with that peer's own HPACK state.
fn headers(enc: &mut Encoder, stream_id: u32, fields: &[HeaderField], end_stream: bool) -> Frame {
    let mut block = Vec::new();
    enc.encode(fields, &mut block);
    Frame::Headers(HeadersFrame {
        stream_id,
        end_stream,
        end_headers: true,
        priority: None,
        header_block_fragment: block,
    })
}

fn data(stream_id: u32, bytes: &[u8], end_stream: bool) -> Frame {
    Frame::Data(DataFrame::new(stream_id, bytes.to_vec(), end_stream))
}

fn settings(id: SettingId, value: u32) -> Frame {
    Frame::Settings(SettingsFrame {
        ack: false,
        settings: vec![Setting { id, value }],
    })
}

fn is_stream_error(r: Result<Vec<Frame>, H2Error>, id: u32, code: ErrorCode) -> bool {
    matches!(r, Err(H2Error::Stream(s, c, _)) if s == id && c == code)
}

/// Decodes the header block of a HEADERS frame this endpoint sent.
fn decode(dec: &mut Decoder, frame: &Frame) -> Vec<HeaderField> {
    match frame {
        Frame::Headers(h) => dec.decode(&h.header_block_fragment).unwrap(),
        other => panic!("expected HEADERS, got {other:?}"),
    }
}

// ---- what a received frame delivers ------------------------------------

#[test]
fn headers_are_delivered_decoded() {
    let (mut c, mut enc) = (server(), Encoder::default());
    c.apply(headers(&mut enc, 1, &request(), false)).unwrap();
    assert_eq!(
        c.take_inbound(),
        vec![Inbound::Headers {
            stream_id: 1,
            fields: request(),
            end_stream: false
        }]
    );
    assert!(c.take_inbound().is_empty(), "taking drains");
}

#[test]
fn data_is_delivered_in_order_with_end_stream() {
    let (mut c, mut enc) = (server(), Encoder::default());
    c.apply(headers(&mut enc, 1, &request(), false)).unwrap();
    c.apply(data(1, b"ab", false)).unwrap();
    c.apply(data(1, b"cd", true)).unwrap();
    let got = c.take_inbound();
    assert_eq!(
        got[1..],
        [
            Inbound::Data {
                stream_id: 1,
                data: b"ab".to_vec(),
                end_stream: false
            },
            Inbound::Data {
                stream_id: 1,
                data: b"cd".to_vec(),
                end_stream: true
            },
        ]
    );
}

#[test]
fn a_second_header_block_is_trailers() {
    let (mut c, mut enc) = (server(), Encoder::default());
    c.apply(headers(&mut enc, 1, &request(), false)).unwrap();
    c.apply(headers(&mut enc, 1, &[hf("grpc-status", "0")], true))
        .unwrap();
    let got = c.take_inbound();
    assert_eq!(
        got[1],
        Inbound::Trailers {
            stream_id: 1,
            fields: vec![hf("grpc-status", "0")]
        }
    );
}

#[test]
fn trailers_that_do_not_end_the_stream_are_a_stream_error() {
    let (mut c, mut enc) = (server(), Encoder::default());
    c.apply(headers(&mut enc, 1, &request(), false)).unwrap();
    let r = c.apply(headers(&mut enc, 1, &[hf("grpc-status", "0")], false));
    assert!(is_stream_error(r, 1, ErrorCode::ProtocolError));
}

#[test]
fn trailers_may_not_carry_pseudo_headers() {
    let (mut c, mut enc) = (server(), Encoder::default());
    c.apply(headers(&mut enc, 1, &request(), false)).unwrap();
    let r = c.apply(headers(&mut enc, 1, &[hf(":status", "200")], true));
    assert!(is_stream_error(r, 1, ErrorCode::ProtocolError));
}

#[test]
fn malformed_header_blocks_are_stream_errors() {
    let mut bad: Vec<Vec<HeaderField>> = Vec::new();
    let mut upper = request();
    upper.push(hf("Content-Type", "x")); // uppercase name (RFC 9113 §8.2.1)
    bad.push(upper);
    let mut late = request();
    late.push(hf(":extra", "x")); // unknown pseudo-header
    bad.push(late);
    let mut order = vec![hf("a", "b")];
    order.extend(request()); // pseudo-header after a regular field
    bad.push(order);
    for fields in bad {
        let (mut c, mut enc) = (server(), Encoder::default());
        let r = c.apply(headers(&mut enc, 1, &fields, false));
        assert!(
            is_stream_error(r, 1, ErrorCode::ProtocolError),
            "{fields:?}"
        );
        assert!(c.take_inbound().is_empty(), "a bad block delivers nothing");
    }
}

#[test]
fn a_rejected_block_still_keeps_hpack_in_sync() {
    let (mut c, mut enc) = (server(), Encoder::default());
    let mut bad = request();
    bad.push(hf("Bad-Name", "x"));
    assert!(c.apply(headers(&mut enc, 1, &bad, false)).is_err());
    // The next block reuses table entries the rejected one added.
    let mut ok = request();
    ok.push(hf("x-trace", "1"));
    c.apply(headers(&mut enc, 3, &ok, false)).unwrap();
    assert_eq!(
        c.take_inbound(),
        vec![Inbound::Headers {
            stream_id: 3,
            fields: ok,
            end_stream: false
        }]
    );
}

#[test]
fn resets_and_goaway_are_delivered() {
    let (mut c, mut enc) = (server(), Encoder::default());
    c.apply(headers(&mut enc, 1, &request(), false)).unwrap();
    c.take_inbound();
    c.apply(Frame::RstStream(RstStreamFrame {
        stream_id: 1,
        error_code: ErrorCode::Cancel,
    }))
    .unwrap();
    c.apply(Frame::GoAway(GoAwayFrame {
        last_stream_id: 1,
        error_code: ErrorCode::NoError,
        debug_data: vec![],
    }))
    .unwrap();
    assert_eq!(
        c.take_inbound(),
        vec![
            Inbound::Reset {
                stream_id: 1,
                code: ErrorCode::Cancel
            },
            Inbound::GoAway {
                last_stream_id: 1,
                code: ErrorCode::NoError
            },
        ]
    );
}

// ---- what a connection sends -------------------------------------------

#[test]
fn every_block_a_connection_sends_shares_one_hpack_state() {
    // One decoder on the far end reads all of them: a fresh encoder per
    // block would desync it after the first.
    let mut c = server();
    let mut dec = Decoder::new(4096);
    let mut enc = Encoder::default();
    for id in [1, 3, 5] {
        c.apply(headers(&mut enc, id, &request(), true)).unwrap();
        let f = c
            .send_headers(
                id,
                &[hf(":status", "200"), hf("content-type", "application/grpc")],
                false,
            )
            .unwrap();
        assert_eq!(decode(&mut dec, &f)[0], hf(":status", "200"));
        let t = c.send_trailers(id, &[hf("grpc-status", "0")]).unwrap();
        assert_eq!(decode(&mut dec, &t), vec![hf("grpc-status", "0")]);
        assert!(matches!(t, Frame::Headers(h) if h.end_stream));
    }
}

#[test]
fn rejected_oversized_headers_do_not_change_hpack_state() {
    let mut c = client();
    let mut dec = Decoder::new(4096);
    let oversized = vec![
        hf(":method", "POST"),
        hf("a", &"x".repeat(65_536)),
        hf("x-small", "v"),
    ];
    assert!(c.open_stream(&oversized, false).is_err());

    let fields = vec![hf(":method", "POST"), hf("x-small", "v")];
    let (_, frame) = c.open_stream(&fields, false).unwrap();
    assert_eq!(decode(&mut dec, &frame), fields);
}

#[test]
fn headers_after_local_half_close_are_rejected_without_changing_hpack() {
    let mut c = client();
    let mut dec = Decoder::new(4096);
    let (closed_id, first) = c.open_stream(&request(), true).unwrap();
    assert_eq!(decode(&mut dec, &first), request());

    assert!(c
        .send_headers(closed_id, &[hf("x-unsent", "v")], false)
        .is_err());
    let fields = vec![hf(":method", "POST"), hf("x-unsent", "v")];
    let (_, next) = c.open_stream(&fields, false).unwrap();
    assert_eq!(decode(&mut dec, &next), fields);
}

#[test]
fn a_client_opens_odd_streams_in_order() {
    let mut c = client();
    let mut dec = Decoder::new(4096);
    let (a, fa) = c.open_stream(&request(), false).unwrap();
    let (b, fb) = c.open_stream(&request(), true).unwrap();
    assert_eq!((a, b), (1, 3));
    assert_eq!(decode(&mut dec, &fa), request());
    assert_eq!(decode(&mut dec, &fb), request());
    assert!(matches!(fb, Frame::Headers(h) if h.end_stream));
}

#[test]
fn data_is_split_at_the_peers_max_frame_size() {
    let mut c = client();
    c.apply(settings(SettingId::MaxFrameSize, 20_000)).unwrap();
    c.apply(Frame::WindowUpdate(WindowUpdateFrame {
        stream_id: 0,
        window_size_increment: 100_000,
    }))
    .unwrap();
    let (id, _) = c.open_stream(&request(), false).unwrap();
    c.apply(Frame::WindowUpdate(WindowUpdateFrame {
        stream_id: id,
        window_size_increment: 100_000,
    }))
    .unwrap();
    let frames = c.send_data(id, &vec![7u8; 50_000], true).unwrap();
    let sizes: Vec<usize> = frames
        .iter()
        .map(|f| match f {
            Frame::Data(d) => d.data.len(),
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(sizes, [20_000, 20_000, 10_000]);
    let ends: Vec<bool> = frames
        .iter()
        .map(|f| matches!(f, Frame::Data(d) if d.end_stream))
        .collect();
    assert_eq!(ends, [false, false, true], "only the last ends the stream");
}

#[test]
fn data_that_does_not_fit_the_window_is_refused_whole() {
    let mut c = client();
    let (id, _) = c.open_stream(&request(), false).unwrap();
    assert!(c.send_data(id, &vec![0u8; 65_536], false).is_err());
    // Nothing was charged: the full window is still available.
    assert!(c.send_data(id, &vec![0u8; 65_535], false).is_ok());
}

// ---- flow-control windows ----------------------------------------------

#[test]
fn a_lowered_initial_window_shrinks_open_streams_send_windows() {
    let mut c = client();
    let (id, _) = c.open_stream(&request(), false).unwrap();
    c.apply(settings(SettingId::InitialWindowSize, 10)).unwrap();
    assert!(c.send_data(id, &[0; 11], false).is_err());
    assert!(c.send_data(id, &[0; 10], false).is_ok());
}

#[test]
fn a_raised_initial_window_grows_open_streams_send_windows() {
    let mut c = client();
    let (id, _) = c.open_stream(&request(), false).unwrap();
    c.apply(Frame::WindowUpdate(WindowUpdateFrame {
        stream_id: 0,
        window_size_increment: 100_000,
    }))
    .unwrap();
    c.apply(settings(SettingId::InitialWindowSize, 100_000))
        .unwrap();
    assert!(c.send_data(id, &vec![0; 100_000], false).is_ok());
}

#[test]
fn a_window_can_go_negative_and_recovers() {
    // §6.9.2: shrinking below what is already in flight leaves a negative
    // window; nothing may be sent until updates bring it back above zero.
    let mut c = client();
    let (id, _) = c.open_stream(&request(), false).unwrap();
    c.send_data(id, &[0; 50], false).unwrap();
    c.apply(settings(SettingId::InitialWindowSize, 10)).unwrap(); // 65_485 - 65_525 = -40
    assert!(c.send_data(id, &[0; 1], false).is_err());
    c.apply(Frame::WindowUpdate(WindowUpdateFrame {
        stream_id: id,
        window_size_increment: 45,
    }))
    .unwrap(); // -40 + 45 = 5
    assert!(c.send_data(id, &[0; 5], false).is_ok());
    assert!(c.send_data(id, &[0; 1], false).is_err());
}

#[test]
fn an_initial_window_over_the_maximum_is_a_connection_error() {
    let mut c = client();
    let r = c.apply(settings(SettingId::InitialWindowSize, 0x8000_0000));
    assert!(matches!(
        r,
        Err(H2Error::Connection(ErrorCode::FlowControlError, _))
    ));
}

#[test]
fn the_receive_window_is_the_one_this_endpoint_advertised() {
    let s = ServerSettings {
        initial_window_size: 1_000,
        ..ServerSettings::default()
    };
    let mut c = Connection::new(s, PeerType::Server);
    c.mark_preface_seen();
    let mut enc = Encoder::default();
    c.apply(headers(&mut enc, 1, &request(), false)).unwrap();
    c.apply(data(1, &[0; 1_000], false)).unwrap();
    // Capacity is returned only when the application releases it.
    let r = c.apply(data(1, &[0; 1], false));
    assert!(is_stream_error(r, 1, ErrorCode::FlowControlError));

    let mut c2 = Connection::new(s, PeerType::Server);
    c2.mark_preface_seen();
    let mut enc2 = Encoder::default();
    c2.apply(headers(&mut enc2, 1, &request(), false)).unwrap();
    let r = c2.apply(data(1, &[0; 1_001], false));
    assert!(is_stream_error(r, 1, ErrorCode::FlowControlError));
}

#[test]
fn released_capacity_becomes_window_updates() {
    let (mut c, mut enc) = (server(), Encoder::default());
    c.apply(headers(&mut enc, 1, &request(), false)).unwrap();
    c.apply(data(1, &[0; 300], false)).unwrap();
    let out = c.release_capacity(1, 300);
    let got: Vec<(u32, u32)> = out
        .iter()
        .map(|f| match f {
            Frame::WindowUpdate(w) => (w.stream_id, w.window_size_increment),
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(got, [(0, 300), (1, 300)]);
}

// ---- server accept ------------------------------------------------------

#[test]
fn poll_accept_yields_each_request_once() {
    use rusty_h2::server::{builder::Builder, connection::Connection as Server};
    use std::task::Poll;

    let mut s = Server::new(Builder::new());
    assert!(matches!(s.poll_accept(), Poll::Pending), "nothing yet");
    let mut enc = Encoder::default();
    s.apply(headers(&mut enc, 1, &request(), false)).unwrap();
    s.apply(headers(&mut enc, 3, &request(), true)).unwrap();
    let Poll::Ready(Some(a)) = s.poll_accept() else {
        panic!("first")
    };
    let Poll::Ready(Some(b)) = s.poll_accept() else {
        panic!("second")
    };
    assert_eq!(
        (a.stream_id(), a.method(), a.path()),
        (1, "POST", "/svc/Method")
    );
    assert_eq!(b.stream_id(), 3);
    assert!(!a.end_stream(), "the first request is awaiting a body");
    assert!(b.end_stream(), "the second request ended with its headers");
    assert!(matches!(s.poll_accept(), Poll::Pending));
}
