//! Message framing: the 5-byte prefix, the receive limit, and arbitrary chunking.
#![allow(clippy::unwrap_used)]

mod common;

use std::time::Instant;

use common::Lcg;
use rusty_prost::rpc::ErrorInfo;
use rusty_tonic::{
    decode_message, encode_frame, encode_message, Code, FrameDecoder,
    DEFAULT_MAX_RECV_MESSAGE_SIZE, HEADER_LEN,
};

fn frames(messages: &[Vec<u8>]) -> Vec<u8> {
    messages
        .iter()
        .flat_map(|m| encode_frame(m).unwrap())
        .collect()
}

fn drain(d: &mut FrameDecoder) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    while let Some(m) = d.next_message().unwrap() {
        out.push(m);
    }
    out
}

#[test]
fn the_prefix_is_a_zero_flag_and_a_big_endian_length() {
    assert_eq!(HEADER_LEN, 5);
    assert_eq!(
        encode_frame(b"abc").unwrap(),
        [0, 0, 0, 0, 3, b'a', b'b', b'c']
    );
    assert_eq!(encode_frame(b"").unwrap(), [0, 0, 0, 0, 0]);
    let long = vec![7u8; 300];
    let f = encode_frame(&long).unwrap();
    assert_eq!(&f[..5], [0, 0, 0, 1, 44], "300 = 0x012c");
    assert_eq!(&f[5..], &long[..]);
}

#[test]
fn a_whole_frame_is_one_message_then_nothing() {
    let mut d = FrameDecoder::default();
    d.push(&encode_frame(b"hello").unwrap());
    assert_eq!(d.next_message().unwrap(), Some(b"hello".to_vec()));
    assert_eq!(d.next_message().unwrap(), None);
    assert_eq!(d.buffered(), 0);
    d.finish().unwrap();
}

#[test]
fn an_empty_message_is_a_message() {
    let mut d = FrameDecoder::default();
    d.push(&encode_frame(b"").unwrap());
    assert_eq!(d.next_message().unwrap(), Some(Vec::new()));
}

#[test]
fn any_chunking_of_a_stream_yields_the_same_messages() {
    let mut rng = Lcg(1);
    for round in 0..300 {
        let msgs: Vec<Vec<u8>> = (0..rng.below(8)).map(|_| rng.bytes(300)).collect();
        let stream = frames(&msgs);
        let mut d = FrameDecoder::default();
        let mut got = Vec::new();
        let mut at = 0;
        while at < stream.len() {
            let cap = if rng.chance() { 3 } else { 200 };
            let n = 1 + rng.below(cap);
            let end = (at + n).min(stream.len());
            d.push(&stream[at..end]);
            at = end;
            got.extend(drain(&mut d));
        }
        assert_eq!(got, msgs, "round {round}");
        d.finish().unwrap();
    }
}

#[test]
fn byte_at_a_time_works() {
    let msgs = vec![b"one".to_vec(), Vec::new(), vec![9; 130]];
    let mut d = FrameDecoder::default();
    let mut got = Vec::new();
    for b in frames(&msgs) {
        d.push(&[b]);
        got.extend(drain(&mut d));
    }
    assert_eq!(got, msgs);
}

#[test]
fn a_partial_message_waits_and_a_cut_stream_is_an_error() {
    let f = encode_frame(b"hello").unwrap();
    for cut in 1..f.len() {
        let mut d = FrameDecoder::default();
        d.push(&f[..cut]);
        assert_eq!(d.next_message().unwrap(), None, "cut {cut}");
        assert_eq!(d.buffered(), cut);
        assert_eq!(d.finish().unwrap_err().code(), Code::Internal, "cut {cut}");
    }
    let mut d = FrameDecoder::default();
    d.push(&f);
    drain(&mut d);
    d.finish().unwrap();
}

#[test]
fn a_compressed_flag_is_unimplemented_and_any_other_flag_is_internal() {
    let mut d = FrameDecoder::default();
    d.push(&[1, 0, 0, 0, 1, 0xff]);
    assert_eq!(d.next_message().unwrap_err().code(), Code::Unimplemented);
    for flag in [2u8, 3, 0x80, 0xff] {
        let mut d = FrameDecoder::default();
        d.push(&[flag, 0, 0, 0, 0]);
        assert_eq!(
            d.next_message().unwrap_err().code(),
            Code::Internal,
            "{flag}"
        );
    }
}

#[test]
fn an_error_is_final() {
    let mut d = FrameDecoder::default();
    d.push(&[9, 0, 0, 0, 0]);
    let first = d.next_message().unwrap_err();
    assert_eq!(d.next_message().unwrap_err(), first);
    d.push(&encode_frame(b"fine").unwrap());
    assert_eq!(
        d.next_message().unwrap_err(),
        first,
        "what is pushed afterwards is ignored"
    );
    assert_eq!(d.finish().unwrap_err(), first);
    assert!(d.buffered() <= HEADER_LEN, "nothing more is held");
}

#[test]
fn a_message_over_the_limit_is_refused_as_soon_as_the_prefix_is_in() {
    let mut d = FrameDecoder::new(10);
    d.push(&[0, 0, 0, 0, 11]); // only the prefix: no body has arrived or been waited for
    let e = d.next_message().unwrap_err();
    assert_eq!(e.code(), Code::ResourceExhausted);
    assert!(
        e.message().contains("11") && e.message().contains("10"),
        "{}",
        e.message()
    );
    // exactly the limit is fine
    let mut d = FrameDecoder::new(10);
    d.push(&encode_frame(&[1u8; 10]).unwrap());
    assert_eq!(d.next_message().unwrap().unwrap().len(), 10);
    // a limit of zero takes only empty messages
    let mut d = FrameDecoder::new(0);
    d.push(&encode_frame(b"").unwrap());
    assert_eq!(d.next_message().unwrap(), Some(Vec::new()));
    d.push(&encode_frame(b"x").unwrap());
    assert_eq!(
        d.next_message().unwrap_err().code(),
        Code::ResourceExhausted
    );
}

#[test]
fn a_huge_claimed_length_costs_nothing() {
    let mut d = FrameDecoder::default();
    d.push(&[0, 0xff, 0xff, 0xff, 0xff]);
    assert_eq!(
        d.next_message().unwrap_err().code(),
        Code::ResourceExhausted
    );
    assert_eq!(DEFAULT_MAX_RECV_MESSAGE_SIZE, 4 * 1024 * 1024);
}

#[test]
fn an_oversized_body_in_the_same_push_is_never_buffered() {
    let mut bytes = vec![0, 0, 0, 0, 11];
    bytes.extend_from_slice(&[7; 1_000_000]);
    let mut d = FrameDecoder::new(10);
    d.push(&bytes);
    assert_eq!(d.buffered(), 0);
    assert_eq!(
        d.next_message().unwrap_err().code(),
        Code::ResourceExhausted
    );
}

#[test]
fn a_split_oversized_prefix_rejects_before_copying_the_body() {
    let mut d = FrameDecoder::new(10);
    d.push(&[0, 0, 0]);
    assert_eq!(d.buffered(), 3);
    let mut rest = vec![0, 11];
    rest.extend_from_slice(&[7; 1_000_000]);
    d.push(&rest);
    assert_eq!(d.buffered(), 0);
    let first = d.next_message().unwrap_err();
    assert_eq!(first.code(), Code::ResourceExhausted);
    assert_eq!(d.next_message().unwrap_err(), first);
}

#[cfg(target_pointer_width = "32")]
#[test]
fn maximum_wire_length_overflow_is_a_terminal_resource_exhausted_error() {
    let mut d = FrameDecoder::new(usize::MAX);
    d.push(&[0, 0xff, 0xff, 0xff, 0xff]);
    let first = d.next_message().unwrap_err();
    assert_eq!(first.code(), Code::ResourceExhausted);
    assert_eq!(d.next_message().unwrap_err(), first);
}

#[test]
fn many_small_messages_in_one_push_are_cheap() {
    let msgs: Vec<Vec<u8>> = (0..100_000u32).map(|i| i.to_be_bytes().to_vec()).collect();
    let mut d = FrameDecoder::default();
    d.push(&frames(&msgs));
    let start = Instant::now();
    let n = drain(&mut d).len();
    assert_eq!(n, 100_000);
    assert!(
        start.elapsed().as_secs() < 5,
        "taking messages off the front must not be quadratic"
    );
    assert_eq!(d.buffered(), 0);
}

#[test]
fn bytes_pushed_after_the_buffer_emptied_start_clean() {
    let mut d = FrameDecoder::default();
    for i in 0..1000u32 {
        d.push(&encode_frame(&i.to_be_bytes()).unwrap());
        assert_eq!(d.next_message().unwrap(), Some(i.to_be_bytes().to_vec()));
    }
    assert_eq!(d.buffered(), 0);
}

#[test]
fn random_bytes_never_panic() {
    let mut rng = Lcg(9);
    for _ in 0..20_000 {
        let mut d = FrameDecoder::new(64);
        for _ in 0..1 + rng.below(4) {
            d.push(&rng.bytes(40));
            while let Ok(Some(_)) = d.next_message() {}
        }
        let _ = d.finish();
    }
}

#[test]
fn corrupting_a_valid_stream_never_panics() {
    let mut rng = Lcg(10);
    let stream = frames(&[
        b"alpha".to_vec(),
        vec![3; 200],
        Vec::new(),
        b"omega".to_vec(),
    ]);
    for _ in 0..20_000 {
        let mut s = stream.clone();
        for _ in 0..1 + rng.below(3) {
            let at = rng.below(s.len());
            s[at] = rng.next() as u8;
        }
        let mut d = FrameDecoder::new(256);
        d.push(&s);
        while let Ok(Some(_)) = d.next_message() {}
        let _ = d.finish();
    }
}

#[test]
fn typed_messages_round_trip_and_a_bad_payload_is_internal() {
    let info = ErrorInfo {
        reason: "R".into(),
        domain: "d".into(),
        ..Default::default()
    };
    let frame = encode_message(&info).unwrap();
    let mut d = FrameDecoder::default();
    d.push(&frame);
    let payload = d.next_message().unwrap().unwrap();
    assert_eq!(decode_message::<ErrorInfo>(&payload).unwrap(), info);
    let e = decode_message::<ErrorInfo>(&[0x0a, 0x09]).unwrap_err();
    assert_eq!(e.code(), Code::Internal);
}
