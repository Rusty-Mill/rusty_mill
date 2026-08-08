//! Unit tests for ClientHello parsing.
//!
//! The messages are built here rather than captured, so a test says which byte
//! it is exercising. Every one of these lengths is attacker-controlled on a
//! real socket, which is what the truncation and overflow cases are about.

use super::*;

/// Wrap a ClientHello body in its handshake and record headers.
fn framed(body: Vec<u8>) -> Vec<u8> {
    let mut handshake = vec![0x01];
    let length = body.len();
    handshake.extend_from_slice(&[
        ((length >> 16) & 0xff) as u8,
        ((length >> 8) & 0xff) as u8,
        (length & 0xff) as u8,
    ]);
    handshake.extend_from_slice(&body);

    let mut record = vec![0x16, 0x03, 0x01];
    record.extend_from_slice(&(handshake.len() as u16).to_be_bytes());
    record.extend_from_slice(&handshake);
    record
}

/// A ClientHello carrying `extensions` verbatim.
fn hello_with(extensions: Vec<u8>) -> Vec<u8> {
    let mut body = vec![0x03, 0x03];
    body.extend_from_slice(&[0u8; 32]); // random
    body.push(0); // no session id
    body.extend_from_slice(&2u16.to_be_bytes()); // one cipher suite
    body.extend_from_slice(&[0x13, 0x01]);
    body.push(1); // one compression method
    body.push(0);
    body.extend_from_slice(&(extensions.len() as u16).to_be_bytes());
    body.extend_from_slice(&extensions);
    framed(body)
}

/// The `server_name` extension for one host.
fn sni(host: &str) -> Vec<u8> {
    let mut name = vec![0x00]; // host_name
    name.extend_from_slice(&(host.len() as u16).to_be_bytes());
    name.extend_from_slice(host.as_bytes());

    let mut list = (name.len() as u16).to_be_bytes().to_vec();
    list.extend_from_slice(&name);

    let mut extension = 0u16.to_be_bytes().to_vec(); // server_name
    extension.extend_from_slice(&(list.len() as u16).to_be_bytes());
    extension.extend_from_slice(&list);
    extension
}

/// An extension of some other kind, to be skipped.
fn other(kind: u16, length: usize) -> Vec<u8> {
    let mut extension = kind.to_be_bytes().to_vec();
    extension.extend_from_slice(&(length as u16).to_be_bytes());
    extension.extend(std::iter::repeat_n(0xab, length));
    extension
}

#[test]
fn a_hello_carrying_a_name_gives_it_back() {
    let hello = hello_with(sni("api.example.com"));
    assert_eq!(server_name(&hello).as_deref(), Some("api.example.com"));
}

#[test]
fn a_name_is_lowercased() {
    // Hostnames are case-insensitive, and a match against a configured one
    // should not depend on how a client typed it.
    let hello = hello_with(sni("API.Example.COM"));
    assert_eq!(server_name(&hello).as_deref(), Some("api.example.com"));
}

#[test]
fn extensions_before_and_after_are_skipped_by_length() {
    // An unknown extension must not stop the search, or a client offering a
    // new one would silently lose SNI selection.
    let mut extensions = other(0x000b, 4);
    extensions.extend(sni("api.example.com"));
    extensions.extend(other(0x0010, 12));
    assert_eq!(
        server_name(&hello_with(extensions)).as_deref(),
        Some("api.example.com")
    );
}

#[test]
fn a_hello_with_no_sni_has_no_name() {
    // Every client addressing the gateway by IP is this case.
    assert_eq!(server_name(&hello_with(other(0x0010, 4))), None);
    assert_eq!(server_name(&hello_with(Vec::new())), None);
}

#[test]
fn a_truncated_message_is_not_a_name() {
    // The common case on a real socket: the peek returned part of the hello.
    let hello = hello_with(sni("api.example.com"));
    for cut in 0..hello.len() {
        assert_eq!(
            server_name(&hello[..cut]),
            None,
            "a hello cut at {cut} bytes must not parse"
        );
    }
    assert!(server_name(&hello).is_some(), "the whole one still parses");
}

#[test]
fn a_length_running_past_the_slice_is_refused_rather_than_panicking() {
    // Every one of these is written by whoever connected.
    let mut hello = hello_with(sni("api.example.com"));

    // The extensions block claims more than the message holds.
    let mut lying = hello.clone();
    let at = lying.len() - sni("api.example.com").len() - 2;
    lying[at] = 0xff;
    lying[at + 1] = 0xff;
    assert_eq!(server_name(&lying), None);

    // The session id claims 255 bytes in a message that has none.
    hello[5 + 4 + 2 + 32] = 0xff;
    assert_eq!(server_name(&hello), None);
}

#[test]
fn something_that_is_not_a_client_hello_is_not_a_name() {
    // A plaintext HTTP request arriving on a TLS port, say.
    assert_eq!(server_name(b"GET / HTTP/1.1\r\nHost: x\r\n\r\n"), None);
    assert_eq!(server_name(&[]), None);

    // A handshake record carrying something other than a ClientHello.
    let mut not_hello = hello_with(sni("api.example.com"));
    not_hello[5] = 0x02; // ServerHello
    assert_eq!(server_name(&not_hello), None);

    // A record that is not a handshake at all.
    let mut not_handshake = hello_with(sni("api.example.com"));
    not_handshake[0] = 0x17; // application data
    assert_eq!(server_name(&not_handshake), None);
}

#[test]
fn a_name_that_is_not_utf8_is_refused() {
    // Lossy conversion could make a hostname match something nobody wrote.
    let mut extension = 0u16.to_be_bytes().to_vec();
    let name: Vec<u8> = vec![0x00, 0x00, 0x02, 0xff, 0xfe];
    let mut list = (name.len() as u16).to_be_bytes().to_vec();
    list.extend_from_slice(&name);
    extension.extend_from_slice(&(list.len() as u16).to_be_bytes());
    extension.extend_from_slice(&list);

    assert_eq!(server_name(&hello_with(extension)), None);
}

#[test]
fn an_empty_name_is_read_as_written() {
    // Not a name anything will match, and not a parse failure either.
    assert_eq!(server_name(&hello_with(sni(""))).as_deref(), Some(""));
}

#[test]
fn a_session_id_and_ticket_do_not_shift_the_name() {
    // A resumed connection carries both, and they sit before the extensions.
    let mut body = vec![0x03, 0x03];
    body.extend_from_slice(&[0u8; 32]);
    body.push(32); // a session id
    body.extend_from_slice(&[0x5a; 32]);
    body.extend_from_slice(&4u16.to_be_bytes());
    body.extend_from_slice(&[0x13, 0x01, 0x13, 0x02]);
    body.push(1);
    body.push(0);

    let mut extensions = other(0x0023, 64); // a session ticket
    extensions.extend(sni("resumed.example.com"));
    body.extend_from_slice(&(extensions.len() as u16).to_be_bytes());
    body.extend_from_slice(&extensions);

    assert_eq!(
        server_name(&framed(body)).as_deref(),
        Some("resumed.example.com")
    );
}
