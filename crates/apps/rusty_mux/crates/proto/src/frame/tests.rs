use rusty_mux_core::{BlockId, ClientId, Event, ExitStatus, Progress, SessionId, Size, WindowId};
use rusty_uuid::Uuid;
use rusty_wire::Writer;

use super::*;
use crate::field::Out;
use crate::schema::Value;
use crate::{Code, Cursor, Entry, Features, Launch, Program};

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
        Message::Checkpoint {
            block,
            offset: u64::MAX,
            total: u64::MAX,
            data: vec![1, 2, 3],
        },
        Message::Hello {
            features: Features::from_bits(0b101),
            resume: None,
        },
        Message::Hello {
            features: Features::EVENT_SEQ,
            resume: Some(Cursor {
                epoch: block.uuid(),
                seq: u64::MAX,
            }),
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
            launch: Launch::default(),
        },
        Message::CreateBlock {
            req: 3,
            window,
            size: size(80, 24),
            launch: Launch {
                program: Some(Program {
                    command: "sh".into(),
                    args: vec!["-c".into(), String::new(), "exit 3".into()],
                }),
                cwd: Some("/tmp/wörk".into()),
            },
        },
        Message::CreateBlock {
            req: 3,
            window,
            size: size(80, 24),
            launch: Launch {
                program: Some(Program {
                    command: "true".into(),
                    args: Vec::new(),
                }),
                cwd: None,
            },
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
        Message::Sync { req: 14 },
        Message::BlockCall {
            req: 15,
            block,
            method: "terminal.format".into(),
            args: vec![
                ("history".into(), Value::Lines(u32::MAX)),
                ("id".into(), Value::Id(Uuid::from_bytes([7; 16]))),
                ("text".into(), Value::Text("\u{4e2d}".into())),
                ("data".into(), Value::Bytes(vec![0, 0xff])),
                ("cols".into(), Value::Cells(u16::MAX)),
            ],
        },
        Message::BlockCall {
            req: 15,
            block,
            method: String::new(),
            args: Vec::new(),
        },
        Message::List { req: 16 },
        Message::Text {
            req: 17,
            text: "$ ls\n\u{4e2d}".into(),
        },
        Message::Entry {
            req: 18,
            entry: Entry::Session {
                session,
                name: "main".into(),
            },
        },
        Message::Entry {
            req: 18,
            entry: Entry::Window {
                window,
                session,
                name: String::new(),
            },
        },
        Message::Entry {
            req: 18,
            entry: Entry::Block {
                block,
                window,
                size: size(80, 24),
                holder: Some(client),
                title: "vim".into(),
            },
        },
        Message::Entry {
            req: 18,
            entry: Entry::Block {
                block,
                window,
                size: size(1, 1),
                holder: None,
                title: String::new(),
            },
        },
        Message::Welcome {
            features: Features::NONE,
            client,
            cursor: None,
        },
        Message::Welcome {
            features: Features::EVENT_SEQ,
            client,
            cursor: Some(Cursor {
                epoch: client.uuid(),
                seq: 0,
            }),
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
            code: None,
        },
        Message::Failed {
            req: 13,
            reason: "unknown block".into(),
            code: Some(Code::NotFound),
        },
        Message::Failed {
            req: 13,
            reason: "no such method".into(),
            code: Some(Code::MethodNotFound),
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
        Event::BlockStatus { block },
        Event::PwdChanged { block },
        Event::TitleChanged { block },
        Event::ProgramExited {
            block,
            status: ExitStatus::Code(-1),
        },
        Event::ProgramExited {
            block,
            status: ExitStatus::Signal(9),
        },
        Event::Bell { block },
        Event::ProgressChanged {
            block,
            progress: Progress::Cleared,
        },
        Event::ProgressChanged {
            block,
            progress: Progress::Normal(100),
        },
        Event::ProgressChanged {
            block,
            progress: Progress::Error(5),
        },
        Event::ProgressChanged {
            block,
            progress: Progress::Indeterminate,
        },
        Event::ProgressChanged {
            block,
            progress: Progress::Paused(0),
        },
        Event::Notification {
            block,
            title: String::new(),
            body: "done; ok".into(),
        },
    ];
    for event in events {
        round_trip(Message::Event {
            seq: None,
            event: event.clone(),
        });
        round_trip(Message::Event {
            seq: Some(7),
            event,
        });
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
    assert!(std::mem::size_of::<crate::field::In<'static>>() <= 1024);
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
            features: Features::NONE,
            resume: None,
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
            features: Features::from_bits(REPEATS),
            resume: None,
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
fn program_event_values_are_checked() {
    let block = BlockId::from_uuid(Uuid::nil());
    let event = |kind: u64, fields: &dyn Fn(&mut Out)| {
        decode(&raw_frame(0x84, |o| {
            o.varint(13, kind).uuid(4, block.uuid());
            fields(o);
        }))
    };
    let invalid = |field| {
        Err(Error::InvalidField {
            message: 0x84,
            field,
        })
    };
    assert_eq!(
        event(18, &|o| {
            o.varint(26, 1).varint(27, 101);
        }),
        invalid(27),
        "over 100%"
    );
    assert_eq!(
        event(18, &|o| {
            o.varint(26, 5);
        }),
        invalid(26),
        "unknown state"
    );
    assert_eq!(
        event(18, &|o| {
            o.varint(26, 4);
        }),
        Err(Error::MissingField {
            message: 0x84,
            field: 27
        }),
        "paused needs a percentage"
    );
    assert_eq!(
        event(16, &|o| {
            o.varint(24, 1 << 32);
        }),
        invalid(24),
        "wider than 32 bits"
    );
    assert_eq!(
        event(16, &|_| {}),
        Err(Error::MissingField {
            message: 0x84,
            field: 25
        }),
        "neither a code nor a signal"
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

#[test]
fn near_limit_unknown_and_duplicate_fields_decode_last_value() {
    for key in [0x1e, 0x14] {
        // The review's legal 16,777,215-byte Hello payload, for both unknown
        // fields and duplicate features. Retained state is checked in field tests.
        let mut frame = Vec::with_capacity(MAX_FRAME as usize + 4);
        frame.extend_from_slice(&[0x00, 0xff, 0xff, 0xff, 0x01]);
        for _ in 0..8_388_606 {
            frame.extend_from_slice(&[key, 0]);
        }
        frame.extend_from_slice(&[0x14, 42]);
        assert_eq!(
            decode(&frame),
            Ok(Some((
                Message::Hello {
                    features: Features::from_bits(42),
                    resume: None,
                },
                frame.len()
            )))
        );
        // A malformed final recognized field must still override valid fields.
        let end = frame.len() - 2;
        frame[end..].copy_from_slice(&[0x15, 0]);
        assert_eq!(
            decode(&frame),
            Err(Error::InvalidField {
                message: 1,
                field: 10
            })
        );
    }
}

#[test]
fn malformed_unknown_field_lengths_varints_and_ids_are_rejected() {
    let mut oversized_id = Writer::new();
    crate::varint::write(&mut oversized_id, (u64::from(u32::MAX) + 1) << 1);
    let cases = [
        (vec![0x1e, 0x80], Error::Truncated),
        ([vec![0x1e], vec![0x80; 10]].concat(), Error::VarintOverflow),
        (vec![0x1f, 0x80], Error::Truncated),
        ([vec![0x1f], vec![0xff; 10]].concat(), Error::VarintOverflow),
        (vec![0x1f, 2, 0], Error::Truncated),
        (
            oversized_id.into_vec(),
            Error::FieldIdTooLarge((u64::from(u32::MAX) + 1) << 1),
        ),
    ];
    for (tail, expected) in cases {
        let mut frame = raw_frame(1, |o| {
            o.varint(10, 0);
        });
        frame.extend(tail);
        let len = (frame.len() - 4) as u32;
        frame[..4].copy_from_slice(&len.to_be_bytes());
        assert_eq!(decode(&frame), Err(expected));
    }
}

#[test]
fn maximum_size_snapshot_still_round_trips() {
    // Type (1), UUID key/length/value (18), data key/length (5).
    let message = Message::Snapshot {
        block: BlockId::from_uuid(Uuid::nil()),
        data: vec![0; MAX_FRAME as usize - 24],
    };
    let frame = encode(&message).unwrap();
    assert_eq!(frame.len(), MAX_FRAME as usize + 4);
    assert_eq!(decode(&frame), Ok(Some((message, frame.len()))));
}

#[test]
fn a_cursor_needs_both_its_fields() {
    const EPOCH_ID: u32 = 17;
    const SEQ_ID: u32 = 18;
    let seq_only = raw_frame(0x01, |o| {
        o.varint(10, 0).varint(SEQ_ID, 3);
    });
    assert_eq!(
        decode(&seq_only),
        Err(Error::MissingField {
            message: 0x01,
            field: EPOCH_ID
        })
    );
    let epoch_only = raw_frame(0x81, |o| {
        o.varint(10, 0)
            .uuid(5, Uuid::nil())
            .uuid(EPOCH_ID, Uuid::nil());
    });
    assert_eq!(
        decode(&epoch_only),
        Err(Error::MissingField {
            message: 0x81,
            field: SEQ_ID
        })
    );
}

#[test]
fn feature_bits_are_distinct() {
    let all = [
        Features::REPLAY_SNAPSHOT,
        Features::CHECKPOINT_V1,
        Features::CHECKPOINT_V2,
        Features::CHECKPOINT_V3,
        Features::EVENT_SEQ,
        Features::SYNC,
        Features::INSPECT,
        Features::BLOCK_CALL,
        Features::STATUS,
        Features::TERMINAL_EVENTS,
        Features::PROGRAM_EVENTS,
        Features::LAUNCH,
    ];
    let mut seen = 0u64;
    for feature in all {
        assert_eq!(feature.bits().count_ones(), 1, "{feature:?}");
        assert_eq!(seen & feature.bits(), 0, "{feature:?} reuses a bit");
        seen |= feature.bits();
    }
}

#[test]
fn the_retired_capture_type_is_unknown() {
    let frame = raw_frame(0x0d, |out| {
        out.varint(REQ_ID, 1).uuid(4, Uuid::new_v4()).varint(19, 5);
    });
    assert_eq!(decode(&frame), Err(Error::UnknownType(0x0d)));
}

#[test]
fn an_unknown_failure_code_decodes_as_none() {
    let frame = raw_frame(0x83, |out| {
        out.varint(REQ_ID, 1).bytes(12, b"why").varint(21, 999);
    });
    assert_eq!(
        decode(&frame).expect("decode").map(|(m, _)| m),
        Some(Message::Failed {
            req: 1,
            reason: "why".into(),
            code: None
        })
    );
}

#[test]
fn malformed_block_call_args_are_invalid() {
    let block = [9u8; 16];
    // A name, then: an unknown tag; a truncated id; text that is not UTF-8;
    // a cell count over u16; a line count over u32; a name that runs past
    // the end.
    let cases: [&[u8]; 6] = [
        b"\x01a\x09",
        b"\x01a\x01\x00\x00",
        b"\x01a\x02\x01\xff",
        b"\x01a\x04\x80\x80\x04",
        b"\x01a\x05\x80\x80\x80\x80\x10",
        b"\x05ab",
    ];
    for args in cases {
        let frame = raw_frame(0x0f, |out| {
            out.varint(REQ_ID, 1)
                .bytes(4, &block)
                .bytes(22, b"terminal.format")
                .bytes(23, args);
        });
        assert!(
            matches!(decode(&frame), Err(Error::InvalidField { field: 23, .. })),
            "{args:?}"
        );
    }
}

#[test]
fn block_call_args_are_bounded_in_count() {
    use crate::args::MAX_ARGS;
    // The smallest argument: an empty name and a zero line count.
    let minimal: &[u8] = b"\x00\x05\x00";
    let frame = |count: usize| {
        raw_frame(0x0f, |out| {
            out.varint(REQ_ID, 1)
                .bytes(4, &[9; 16])
                .bytes(22, b"terminal.format")
                .bytes(23, &minimal.repeat(count));
        })
    };
    let Ok(Some((Message::BlockCall { args, .. }, _))) = decode(&frame(MAX_ARGS)) else {
        panic!("{MAX_ARGS} arguments decode");
    };
    assert_eq!(args.len(), MAX_ARGS);
    // One more, or a frame's worth, is refused rather than materialized.
    for count in [MAX_ARGS + 1, 1 << 20] {
        assert!(
            matches!(
                decode(&frame(count)),
                Err(Error::InvalidField { field: 23, .. })
            ),
            "{count}"
        );
    }
}

#[test]
fn launch_arguments_need_a_command_and_well_formed_texts() {
    let window = [9u8; 16];
    let create = |body: &dyn Fn(&mut Out)| {
        raw_frame(0x04, |out| {
            out.varint(REQ_ID, 1)
                .bytes(3, &window)
                .varint(7, 80)
                .varint(8, 24);
            body(out);
        })
    };
    // Arguments without a command; one running past the end; one not UTF-8.
    let cases: [&dyn Fn(&mut Out); 3] = [
        &|out| {
            out.bytes(30, b"\x01a");
        },
        &|out| {
            out.bytes(29, b"sh").bytes(30, b"\x05ab");
        },
        &|out| {
            out.bytes(29, b"sh").bytes(30, b"\x01\xff");
        },
    ];
    for body in cases {
        assert!(matches!(
            decode(&create(body)),
            Err(Error::InvalidField { field: 30, .. })
        ));
    }
    // A command alone runs with no arguments.
    let Ok(Some((Message::CreateBlock { launch, .. }, _))) = decode(&create(&|out| {
        out.bytes(29, b"true");
    })) else {
        panic!("a command alone decodes");
    };
    assert_eq!(
        launch.program,
        Some(Program {
            command: "true".into(),
            args: Vec::new()
        })
    );
}

#[test]
fn block_call_words_round_trip_and_are_bounded() {
    round_trip(Message::BlockCall {
        req: 1,
        block: BlockId::from_uuid(Uuid::nil()),
        method: "terminal.format".into(),
        args: vec![(
            "words".into(),
            Value::Words(vec!["a".into(), String::new()]),
        )],
    });
    // A count larger than the bytes left is refused before reading.
    let frame = raw_frame(0x0f, |out| {
        out.varint(REQ_ID, 1)
            .bytes(4, &[9; 16])
            .bytes(22, b"terminal.format")
            .bytes(23, b"\x01a\x06\xff\xff\xff\x0f");
    });
    assert!(matches!(
        decode(&frame),
        Err(Error::InvalidField { field: 23, .. })
    ));
}
