//! Integration tests: full stack verification across HPACK, frame codec, and connection driver.
//! Exercises HPACK -> Frame encode/decode -> Frame enum roundtrip across the full pipeline.

use rusty_h2::frame::header::{Flags, DEFAULT_MAX_FRAME_SIZE, FRAME_HEADER_LEN};
use rusty_h2::frame::{
    DataFrame, Frame, FrameHeader, FrameType, HeadersFrame, PingFrame, SettingsFrame,
};
use rusty_h2::hpack::{Decoder, Encoder, HeaderField};

/// Verify HPACK encoding roundtrip for a GET request.
#[test]
fn get_headers_roundtrip() {
    let mut encoder = Encoder::default();
    let headers = vec![
        HeaderField::new(":method", "GET"),
        HeaderField::new(":scheme", "https"),
        HeaderField::new(":authority", "example.com"),
        HeaderField::new(":path", "/index.html"),
        HeaderField::new("user-agent", "rusty_h2/0.1"),
    ];
    let mut header_block = Vec::new();
    encoder.encode(&headers, &mut header_block);
    assert!(!header_block.is_empty());

    let mut decoder = Decoder::new(4096);
    let decoded = decoder.decode(&header_block).unwrap();
    assert_eq!(decoded.len(), 5);
    assert_eq!(decoded[0].name, b":method");
    assert_eq!(decoded[0].value, b"GET");
    assert_eq!(decoded[1].name, b":scheme");
    assert_eq!(decoded[1].value, b"https");
    assert_eq!(decoded[2].name, b":authority");
    assert_eq!(decoded[2].value, b"example.com");
    assert_eq!(decoded[3].name, b":path");
    assert_eq!(decoded[3].value, b"/index.html");
}

/// Verify HPACK encoding roundtrip for a POST request with body.
#[test]
fn post_headers_roundtrip() {
    let mut encoder = Encoder::default();
    let headers = vec![
        HeaderField::new(":method", "POST"),
        HeaderField::new(":scheme", "http"),
        HeaderField::new(":authority", "api.example.com"),
        HeaderField::new(":path", "/v1/users"),
        HeaderField::new("content-type", "application/json"),
        HeaderField::sensitive("authorization", "Bearer token123"),
    ];
    let mut header_block = Vec::new();
    encoder.encode(&headers, &mut header_block);
    assert!(!header_block.is_empty());

    let mut decoder = Decoder::new(4096);
    let decoded = decoder.decode(&header_block).unwrap();
    assert_eq!(decoded.len(), 6);
    assert_eq!(decoded[0].name, b":method");
    assert_eq!(decoded[0].value, b"POST");
    assert_eq!(decoded[4].name, b"content-type");
    assert_eq!(decoded[4].value, b"application/json");
    assert!(decoded[5].sensitive);
}

/// HEADERS frame + HPACK body: encode frame -> wire -> decode frame -> decode HPACK.
#[test]
fn headers_frame_with_hpack_roundtrip() {
    let mut encoder = Encoder::default();
    let headers = vec![
        HeaderField::new(":method", "GET"),
        HeaderField::new(":path", "/"),
    ];
    let mut header_block = Vec::new();
    encoder.encode(&headers, &mut header_block);

    let headers_frame = HeadersFrame {
        stream_id: 1,
        header_block_fragment: header_block,
        end_stream: true,
        end_headers: true,
        priority: None,
    };

    let mut wire = Vec::new();
    headers_frame.encode(&mut wire);
    assert!(
        wire.len() >= FRAME_HEADER_LEN,
        "frame should have header at least"
    );

    let hdr = FrameHeader::decode(&wire).unwrap();
    assert_eq!(hdr.frame_type, FrameType::Headers);
    assert_eq!(hdr.stream_id, 1);
    assert!(hdr.flags.contains(Flags::END_STREAM));
    assert!(hdr.flags.contains(Flags::END_HEADERS));

    let decoded_frame = Frame::decode(&hdr, &wire[FRAME_HEADER_LEN..]).unwrap();
    let decoded_headers = match decoded_frame {
        Frame::Headers(f) => f,
        _ => panic!("expected Headers frame"),
    };

    let mut decoder = Decoder::new(4096);
    let decoded = decoder
        .decode(&decoded_headers.header_block_fragment)
        .unwrap();
    assert_eq!(decoded.len(), 2);
    assert_eq!(decoded[0].name, b":method");
    assert_eq!(decoded[0].value, b"GET");
    assert_eq!(decoded[1].name, b":path");
    assert_eq!(decoded[1].value, b"/");
}

/// DATA frame roundtrip: encode -> wire -> decode -> verify payload and flags.
#[test]
fn data_frame_roundtrip() {
    let data = b"Hello, HTTP/2!";
    let data_frame = DataFrame {
        stream_id: 3,
        data: data.to_vec(),
        end_stream: false,
    };

    let mut wire = Vec::new();
    data_frame.encode(&mut wire);

    let hdr = FrameHeader::decode(&wire).unwrap();
    assert_eq!(hdr.frame_type, FrameType::Data);
    assert_eq!(hdr.stream_id, 3);
    assert_eq!(hdr.length as usize, data.len());

    let decoded_frame = Frame::decode(&hdr, &wire[FRAME_HEADER_LEN..]).unwrap();
    let decoded_data = match decoded_frame {
        Frame::Data(f) => f,
        _ => panic!("expected Data frame"),
    };
    assert_eq!(decoded_data.data, data);
    assert!(!decoded_data.end_stream);
}

/// Ping frame roundtrip: encode -> wire -> decode -> verify opaque data.
#[test]
fn ping_frame_roundtrip() {
    let ping_data: [u8; 8] = [1, 2, 3, 4, 5, 6, 7, 8];
    let frame = PingFrame {
        ack: false,
        opaque_data: ping_data,
    };

    let mut wire = Vec::new();
    frame.encode(&mut wire);

    let hdr = FrameHeader::decode(&wire).unwrap();
    assert_eq!(hdr.frame_type, FrameType::Ping);
    assert_eq!(hdr.length, 8);
    assert_eq!(hdr.stream_id, 0);

    let decoded_frame = Frame::decode(&hdr, &wire[FRAME_HEADER_LEN..]).unwrap();
    let decoded_ping = match decoded_frame {
        Frame::Ping(f) => f,
        _ => panic!("expected Ping frame"),
    };
    assert_eq!(decoded_ping.opaque_data, ping_data);
}

/// SETTINGS frame roundtrip: encode -> wire -> decode -> verify settings.
#[test]
fn settings_frame_roundtrip() {
    let settings = vec![
        rusty_h2::frame::settings::Setting {
            id: rusty_h2::frame::settings::SettingId::HeaderTableSize,
            value: 4096,
        },
        rusty_h2::frame::settings::Setting {
            id: rusty_h2::frame::settings::SettingId::InitialWindowSize,
            value: 65535,
        },
    ];
    let frame = SettingsFrame::new(settings.clone());

    let mut wire = Vec::new();
    frame.encode(&mut wire);

    let hdr = FrameHeader::decode(&wire).unwrap();
    assert_eq!(hdr.frame_type, FrameType::Settings);
    assert_eq!(hdr.stream_id, 0);
    assert!(
        (hdr.length as usize).is_multiple_of(6),
        "settings payload must be 6-byte multiples"
    );

    let decoded_frame = Frame::decode(&hdr, &wire[FRAME_HEADER_LEN..]).unwrap();
    match decoded_frame {
        Frame::Settings(f) => assert_eq!(f.settings, settings),
        _ => panic!("expected Settings frame"),
    };
}

/// All frame types roundtrip: encode -> wire -> decode -> verify equality.
#[test]
fn frame_enum_roundtrip_preserves_all_types() {
    let test_cases: Vec<Frame> = vec![
        Frame::Data(DataFrame {
            stream_id: 1,
            data: b"test".to_vec(),
            end_stream: false,
        }),
        Frame::Headers(HeadersFrame {
            stream_id: 3,
            header_block_fragment: vec![0x80],
            end_stream: false,
            end_headers: true,
            priority: None,
        }),
        Frame::Ping(PingFrame {
            ack: false,
            opaque_data: [0; 8],
        }),
    ];

    for original in test_cases {
        let mut wire = Vec::new();
        original.encode(&mut wire);
        let hdr = FrameHeader::decode(&wire).unwrap();
        let decoded = Frame::decode(&hdr, &wire[FRAME_HEADER_LEN..]).unwrap();
        assert_eq!(
            original, decoded,
            "roundtrip failed for {:?}",
            hdr.frame_type
        );
    }
}

/// Frame header: verify encoding and decode match expected wire format.
#[test]
fn frame_header_wire_format() {
    let h = FrameHeader::new(
        DEFAULT_MAX_FRAME_SIZE,
        FrameType::Headers,
        Flags::END_STREAM | Flags::END_HEADERS,
        0x7fff_ffff,
    );

    let mut wire = Vec::new();
    h.encode(&mut wire);
    assert_eq!(wire.len(), FRAME_HEADER_LEN);

    let decoded = FrameHeader::decode(&wire).unwrap();
    assert_eq!(decoded.length, DEFAULT_MAX_FRAME_SIZE);
    assert_eq!(decoded.frame_type, FrameType::Headers);
    assert_eq!(decoded.stream_id, 0x7fff_ffff);
    assert!(decoded.flags.contains(Flags::END_STREAM));
    assert!(decoded.flags.contains(Flags::END_HEADERS));
}

/// Frame size limit: verify DEFAULT_MAX_FRAME_SIZE is respected.
#[test]
fn default_max_frame_size() {
    assert_eq!(DEFAULT_MAX_FRAME_SIZE, 16384);
    assert_eq!(FRAME_HEADER_LEN, 9);
}

/// HPACK dynamic table: repeated headers should use indexed representation.
#[test]
fn hpack_repeated_header_uses_indexed() {
    let mut encoder = Encoder::default();

    // First encoding - header goes into dynamic table
    let mut first = Vec::new();
    let hdr1 = vec![HeaderField::new("x-custom", "value1")];
    encoder.encode(&hdr1, &mut first);

    // Second encoding - the same header should now be found in dynamic table
    // and use indexed representation (byte with 0x80 bit set)
    let mut second = Vec::new();
    let hdr2 = vec![HeaderField::new("x-custom", "value1")];
    encoder.encode(&hdr2, &mut second);

    assert_eq!(second.len(), 1);
    assert!(second[0] & 0x80 != 0, "should use indexed representation");
}

/// HPACK sensitive headers should never be stored in dynamic table.
#[test]
fn hpack_sensitive_never_indexed() {
    let mut encoder = Encoder::default();
    let headers = vec![HeaderField::sensitive("authorization", "secret")];
    let mut block = Vec::new();
    encoder.encode(&headers, &mut block);

    assert!(
        block[0] & 0x10 != 0,
        "sensitive header should use literal never-indexed"
    );

    let mut decoder = Decoder::new(4096);
    let decoded = decoder.decode(&block).unwrap();
    assert!(decoded[0].sensitive);

    // Encode again - should still be literal (not indexed), because never-indexed does not grow dynamic table
    let mut block2 = Vec::new();
    encoder.encode(&headers, &mut block2);
    assert!(
        block2[0] & 0x10 != 0,
        "second encoding should still be literal"
    );
    assert_ne!(block2[0], 0x81, "should not use indexed representation");
}

/// Connection preface: verify it is exactly 24 octets as specified in RFC 9113.
#[test]
fn connection_preface_length() {
    use rusty_h2::CONNECTION_PREFACE;
    assert_eq!(CONNECTION_PREFACE.len(), 24);
}

/// Verify that the connection preface starts with "PRI * HTTP/2.0".
#[test]
fn connection_preface_magic() {
    use rusty_h2::CONNECTION_PREFACE;
    assert!(
        CONNECTION_PREFACE.starts_with(b"PRI * HTTP/2.0"),
        "preface should start with magic string"
    );
}
