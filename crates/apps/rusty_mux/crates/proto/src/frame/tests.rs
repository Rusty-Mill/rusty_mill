use rusty_mux_core::{BlockId, ClientId, Event, SessionId, Size, WindowId};
use rusty_uuid::Uuid;
use rusty_wire::Writer;

use super::*;
use crate::Features;
use crate::field::Out;

const REQ_ID: u32 = 1;

fn size(cols: u16, rows: u16) -> Size {
    Size::new(cols, rows).expect("non-zero size")
}

fn round_trip(message: Message) {
    let frame = encode(&message).expect("encode");
    let decoded = decode(&frame).expect("decode").expect("whole frame");
    assert_eq!(decoded, (message, frame.len()));
}

/// A frame built by hand, for bodies `encode` would never produce.
fn raw_frame(ty: u8, body: impl FnOnce(&mut Out)) -> Vec<u8> {
    let mut out = Out(Writer::new());
    body(&mut out);
    let body = out.0.into_vec();
    let mut frame = ((body.len() + 1) as u32).to_be_bytes().to_vec();
    frame.push(ty);
    frame.extend(body);
    frame
}

#[test]
fn every_message_round_trips() {
    let session = SessionId::from_uuid(Uuid::new_v4());
    let window = WindowId::from_uuid(Uuid::new_v4());
    let block = BlockId::from_uuid(Uuid::new_v4());
    let client = ClientId::from_uuid(Uuid::new_v4());
    let messages = [
        Message::Hello {
            features: Features::from_bits(0b101),
        },
        Message::CreateSession {
            req: 1,
            name: "wörk".into(),
        },
        Message::CreateWindow {
            req: 2,
            session,
            name: String::new(),
        },
        Message::CreateBlock {
            req: 3,
            window,
            size: size(80, 24),
        },
        Message::Attach { req: 4, block },
        Message::Detach { req: 5 },
        Message::AcquireLease { req: 6, block },
        Message::ReleaseLease { req: 7, block },
        Message::Input {
            req: u64::MAX,
            block,
            data: b"ls\r".to_vec(),
        },
        Message::Resize {
            req: 9,
            block,
            size: size(u16::MAX, 1),
        },
        Message::CloseBlock { req: 10, block },
        Message::Welcome {
            features: Features::NONE,
            client,
        },
        Message::Done {
            req: 11,
            created: Some(block.uuid()),
        },
        Message::Done {
            req: 12,
            created: None,
        },
        Message::Failed {
            req: 13,
            reason: "no lease".into(),
        },
        Message::Snapshot {
            block,
            data: vec![0; 70_000],
        },
        Message::Ready { block },
        Message::Output {
            block,
            data: Vec::new(),
        },
    ];
    for message in messages {
        round_trip(message);
    }
}

#[test]
fn every_event_round_trips() {
    let session = SessionId::from_uuid(Uuid::new_v4());
    let window = WindowId::from_uuid(Uuid::new_v4());
    let block = BlockId::from_uuid(Uuid::new_v4());
    let client = ClientId::from_uuid(Uuid::new_v4());
    let events = [
        Event::SessionCreated { session },
        Event::SessionClosed { session },
        Event::WindowCreated { window, session },
        Event::WindowClosed { window },
        Event::BlockCreated {
            block,
            window,
            size: size(80, 24),
        },
        Event::BlockResized {
            block,
            size: size(100, 30),
        },
        Event::BlockClosed { block },
        Event::ClientConnected { client },
        Event::ClientDisconnected { client },
        Event::Attached { client, block },
        Event::Detached { client, block },
        Event::LeaseChanged {
            block,
            holder: Some(client),
        },
        Event::LeaseChanged {
            block,
            holder: None,
        },
    ];
    for event in events {
        round_trip(Message::Event(event));
    }
}

#[test]
fn partial_frames_wait_for_more_bytes() {
    let frame = encode(&Message::Detach { req: 300 }).unwrap();
    for cut in 0..frame.len() {
        assert_eq!(decode(&frame[..cut]), Ok(None), "cut at {cut}");
    }
}

#[test]
fn decode_reports_only_the_first_frame() {
    let mut stream = encode(&Message::Detach { req: 1 }).unwrap();
    let first = stream.len();
    stream.extend(encode(&Message::Detach { req: 2 }).unwrap());
    let (message, used) = decode(&stream).unwrap().unwrap();
    assert_eq!((message, used), (Message::Detach { req: 1 }, first));
    assert_eq!(
        decode(&stream[used..]).unwrap().unwrap().0,
        Message::Detach { req: 2 }
    );
}

#[test]
fn unknown_fields_are_skipped() {
    let frame = raw_frame(0x06, |o| {
        o.bytes(99, b"from a newer peer")
            .varint(REQ_ID, 7)
            .varint(100, 1);
    });
    assert_eq!(
        decode(&frame).unwrap().unwrap().0,
        Message::Detach { req: 7 }
    );
}

/// Fields repeated past any real use: one per id is kept, so decode state is
/// fixed whatever the count.
const REPEATS: u64 = 1 << 20;
const FEATURES_ID: u32 = 10;

#[test]
fn decode_state_is_bounded_by_ids_not_by_field_count() {
    // `In` owns nothing to free, so it cannot grow with the frame.
    assert!(!std::mem::needs_drop::<crate::field::In<'static>>());
    assert!(std::mem::size_of::<crate::field::In<'static>>() <= 512);
    // Unknown ids, both below and above the kept range.
    let unknown = raw_frame(0x01, |o| {
        for i in 0..REPEATS {
            o.varint(15, i).varint(99, i).bytes(1000, b"x");
        }
        o.varint(FEATURES_ID, 0);
    });
    assert_eq!(
        decode(&unknown).unwrap().unwrap().0,
        Message::Hello {
            features: Features::NONE
        }
    );
    // A known id repeated: the last occurrence wins.
    let repeated = raw_frame(0x01, |o| {
        for i in 0..=REPEATS {
            o.varint(FEATURES_ID, i);
        }
    });
    assert_eq!(
        decode(&repeated).unwrap().unwrap().0,
        Message::Hello {
            features: Features::from_bits(REPEATS)
        }
    );
}

#[test]
fn the_last_occurrence_of_a_field_is_the_one_validated() {
    let frame = raw_frame(0x01, |o| {
        o.varint(FEATURES_ID, 1).bytes(FEATURES_ID, b"x");
    });
    assert_eq!(
        decode(&frame),
        Err(Error::InvalidField {
            message: 0x01,
            field: FEATURES_ID
        })
    );
}

#[test]
fn malformed_unknown_fields_are_still_rejected() {
    // Unknown id 99 as a byte string whose length runs past the frame.
    assert_eq!(
        decode(&[0, 0, 0, 4, 0x06, 0xc7, 0x01, 0x05]),
        Err(Error::Truncated)
    );
    // Unknown id 99 as a varint cut off mid-value.
    assert_eq!(
        decode(&[0, 0, 0, 4, 0x06, 0xc6, 0x01, 0x80]),
        Err(Error::Truncated)
    );
}

#[test]
fn oversized_length_is_rejected_from_the_prefix_alone() {
    let prefix = (MAX_FRAME + 1).to_be_bytes();
    assert_eq!(decode(&prefix), Err(Error::FrameTooLarge(MAX_FRAME + 1)));
    let big = Message::Output {
        block: BlockId::from_uuid(Uuid::nil()),
        data: vec![0; MAX_FRAME as usize],
    };
    assert!(matches!(encode(&big), Err(Error::FrameTooLarge(_))));
}

#[test]
fn malformed_frames_are_rejected() {
    assert_eq!(decode(&[0, 0, 0, 0]), Err(Error::EmptyFrame));
    assert_eq!(
        decode(&raw_frame(0x7f, |_| {})),
        Err(Error::UnknownType(0x7f))
    );
    assert_eq!(
        decode(&raw_frame(0x06, |_| {})),
        Err(Error::MissingField {
            message: 0x06,
            field: REQ_ID
        })
    );
    assert_eq!(
        decode(&raw_frame(0x06, |o| {
            o.bytes(REQ_ID, b"7");
        })),
        Err(Error::InvalidField {
            message: 0x06,
            field: REQ_ID
        })
    );
    // A byte-string field whose length runs past the frame.
    assert_eq!(
        decode(&[0, 0, 0, 3, 0x06, 0x03, 0x05]),
        Err(Error::Truncated)
    );
}

#[test]
fn invalid_values_are_rejected() {
    let block = BlockId::from_uuid(Uuid::nil());
    let zero_cols = raw_frame(0x0a, |o| {
        o.varint(REQ_ID, 1)
            .uuid(4, block.uuid())
            .varint(7, 0)
            .varint(8, 24);
    });
    assert_eq!(
        decode(&zero_cols),
        Err(Error::InvalidField {
            message: 0x0a,
            field: 7
        })
    );
    let wide_cols = raw_frame(0x0a, |o| {
        o.varint(REQ_ID, 1)
            .uuid(4, block.uuid())
            .varint(7, 70_000)
            .varint(8, 24);
    });
    assert_eq!(
        decode(&wide_cols),
        Err(Error::InvalidField {
            message: 0x0a,
            field: 7
        })
    );
    let short_uuid = raw_frame(0x05, |o| {
        o.varint(REQ_ID, 1).bytes(4, &[1, 2, 3]);
    });
    assert_eq!(
        decode(&short_uuid),
        Err(Error::InvalidField {
            message: 0x05,
            field: 4
        })
    );
    let bad_utf8 = raw_frame(0x02, |o| {
        o.varint(REQ_ID, 1).bytes(6, &[0xff]);
    });
    assert_eq!(
        decode(&bad_utf8),
        Err(Error::InvalidField {
            message: 0x02,
            field: 6
        })
    );
    let bad_kind = raw_frame(0x84, |o| {
        o.varint(13, 99);
    });
    assert_eq!(
        decode(&bad_kind),
        Err(Error::InvalidField {
            message: 0x84,
            field: 13
        })
    );
}

#[test]
fn features_intersect() {
    let ours = Features::from_bits(0b0110);
    let theirs = Features::from_bits(0b0011);
    assert_eq!(ours.intersection(theirs), Features::from_bits(0b0010));
    assert!(ours.contains(Features::from_bits(0b0100)));
    assert!(!ours.contains(theirs));
    assert!(ours.contains(Features::NONE));
}
