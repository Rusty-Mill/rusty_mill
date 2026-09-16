//! Integration tests using the canonical request-sequence test vectors from
//! RFC 7541 Appendix C.3 (literal encoding) and Appendix C.4 (Huffman
//! encoding). Each sequence is decoded with a single `Decoder` instance so
//! the dynamic table evolves exactly as the RFC describes.

use rusty_h2::hpack::{Decoder, HeaderField};

fn hex(s: &str) -> Vec<u8> {
    let s: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

fn h(name: &str, value: &str) -> HeaderField {
    HeaderField::new(name, value)
}

/// RFC 7541 Appendix C.3: three requests, literal (non-Huffman) encoding.
#[test]
fn appendix_c3_request_examples_without_huffman() {
    let mut dec = Decoder::default();

    // C.3.1
    let block = hex("828684410f7777772e6578616d706c652e636f6d");
    let headers = dec.decode(&block).unwrap();
    assert_eq!(
        headers,
        vec![
            h(":method", "GET"),
            h(":scheme", "http"),
            h(":path", "/"),
            h(":authority", "www.example.com")
        ]
    );
    assert_eq!(dec.dynamic_table_len(), 1);
    assert_eq!(dec.dynamic_table_size(), 57);

    // C.3.2
    let block = hex("828684be58086e6f2d6361636865");
    let headers = dec.decode(&block).unwrap();
    assert_eq!(
        headers,
        vec![
            h(":method", "GET"),
            h(":scheme", "http"),
            h(":path", "/"),
            h(":authority", "www.example.com"),
            h("cache-control", "no-cache"),
        ]
    );
    assert_eq!(dec.dynamic_table_len(), 2);
    assert_eq!(dec.dynamic_table_size(), 110);

    // C.3.3
    let block = hex("828785bf400a637573746f6d2d6b65790c637573746f6d2d76616c7565");
    let headers = dec.decode(&block).unwrap();
    assert_eq!(
        headers,
        vec![
            h(":method", "GET"),
            h(":scheme", "https"),
            h(":path", "/index.html"),
            h(":authority", "www.example.com"),
            h("custom-key", "custom-value"),
        ]
    );
    assert_eq!(dec.dynamic_table_len(), 3);
    assert_eq!(dec.dynamic_table_size(), 164);
}

/// RFC 7541 Appendix C.4: the same three requests, Huffman-encoded.
#[test]
fn appendix_c4_request_examples_with_huffman() {
    let mut dec = Decoder::default();

    // C.4.1
    let block = hex("828684418cf1e3c2e5f23a6ba0ab90f4ff");
    let headers = dec.decode(&block).unwrap();
    assert_eq!(
        headers,
        vec![
            h(":method", "GET"),
            h(":scheme", "http"),
            h(":path", "/"),
            h(":authority", "www.example.com")
        ]
    );
    assert_eq!(dec.dynamic_table_size(), 57);

    // C.4.2
    let block = hex("828684be5886a8eb10649cbf");
    let headers = dec.decode(&block).unwrap();
    assert_eq!(
        headers,
        vec![
            h(":method", "GET"),
            h(":scheme", "http"),
            h(":path", "/"),
            h(":authority", "www.example.com"),
            h("cache-control", "no-cache"),
        ]
    );
    assert_eq!(dec.dynamic_table_size(), 110);

    // C.4.3
    let block = hex("828785bf408825a849e95ba97d7f8925a849e95bb8e8b4bf");
    let headers = dec.decode(&block).unwrap();
    assert_eq!(
        headers,
        vec![
            h(":method", "GET"),
            h(":scheme", "https"),
            h(":path", "/index.html"),
            h(":authority", "www.example.com"),
            h("custom-key", "custom-value"),
        ]
    );
    assert_eq!(dec.dynamic_table_size(), 164);
}

/// Encoding a header list and immediately decoding it back must always
/// round-trip, for arbitrary-ish header sets, independent of the RFC's own
/// fixed examples above.
#[test]
fn encoder_decoder_roundtrip_realistic_request() {
    let mut enc = rusty_h2::hpack::Encoder::default();
    let mut dec = Decoder::default();

    let requests = [
        vec![
            h(":method", "GET"),
            h(":scheme", "https"),
            h(":path", "/"),
            h(":authority", "example.com"),
            h("user-agent", "rusty_h2/0.1"),
        ],
        vec![
            h(":method", "POST"),
            h(":scheme", "https"),
            h(":path", "/submit"),
            h(":authority", "example.com"),
            h("content-type", "application/json"),
            HeaderField::sensitive("authorization", "Bearer secret-token"),
        ],
    ];

    for headers in &requests {
        let mut block = Vec::new();
        enc.encode(headers, &mut block);
        let decoded = dec.decode(&block).unwrap();
        assert_eq!(&decoded, headers);
    }
}
