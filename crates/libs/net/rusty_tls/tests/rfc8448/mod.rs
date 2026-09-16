//! RFC 8448 §3, "A Simple 1-RTT Handshake" — the shared test vector.
//!
//! Two test binaries need these bytes: `handrolled_handshake` asserts what
//! they parse to, and `handrolled_fuzz` mutates them. They live here rather
//! than in both because a hand-copied second set of hex constants is a set
//! that drifts, and a fuzzer seeded from a corrupted corpus reports nothing
//! while looking busy.
//!
//! Everything here was extracted mechanically from the RFC text rather than
//! transcribed — an earlier stage of this work lost an afternoon to a
//! hand-typed `c9` that should have been `9c`.
//!
//! Not every constant is used by every consumer, hence the crate-level
//! `dead_code` tolerance below: this is one corpus, not two subsets.

#![allow(dead_code)]

/// Decode hex, ignoring whitespace and layout.
pub fn hex(text: &str) -> Vec<u8> {
    let digits: Vec<char> = text.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    assert_eq!(digits.len() % 2, 0, "hex input has an odd number of digits");
    digits
        .chunks(2)
        .map(|pair| u8::from_str_radix(&pair.iter().collect::<String>(), 16).expect("hex"))
        .collect()
}

/// The client's ClientHello: 196 octets.
pub const CLIENT_HELLO: &str = "\
    010000c00303cb34ecb1e78163ba1c38\
    c6dacb196a6dffa21a8d9912ec18a2ef\
    6283024dece700000613011303130201\
    0000910000000b000900000673657276\
    6572ff01000100000a00140012001d00\
    17001800190100010101020103010400\
    230000003300260024001d002099381d\
    e560e4bd43d23d8e435a7dbafeb3c06e\
    51c13cae4d5413691e529aaf2c002b00\
    03020304000d0020001e040305030603\
    02030804080508060401050106010201\
    0402050206020202002d00020101001c\
    00024001\
    ";

/// The server's ServerHello: 90 octets.
pub const SERVER_HELLO: &str = "\
    020000560303a6af06a4121860dc5e6e\
    60249cd34c95930c8ac5cb1434dac155\
    772ed3e2692800130100002e00330024\
    001d0020c9828876112095fe66762bdb\
    f7c672e156d6cc253b833df1dd69b1b0\
    4e751f0f002b00020304\
    ";

/// The server's encrypted flight: EncryptedExtensions, Certificate,
/// CertificateVerify, Finished — four messages concatenated in one record,
/// 657 octets.
pub const SERVER_FLIGHT: &str = "\
    080000240022000a00140012001d0017\
    0018001901000101010201030104001c\
    00024001000000000b0001b9000001b5\
    0001b0308201ac30820115a003020102\
    020102300d06092a864886f70d01010b\
    0500300e310c300a0603550403130372\
    7361301e170d31363037333030313233\
    35395a170d3236303733303031323335\
    395a300e310c300a0603550403130372\
    736130819f300d06092a864886f70d01\
    0101050003818d0030818902818100b4\
    bb498f8279303d980836399b36c6988c\
    0c68de55e1bdb826d3901a2461eafd2d\
    e49a91d015abbc9a95137ace6c1af19e\
    aa6af98c7ced43120998e187a80ee0cc\
    b0524b1b018c3e0b63264d449a6d38e2\
    2a5fda430846748030530ef0461c8ca9\
    d9efbfae8ea6d1d03e2bd193eff0ab9a\
    8002c47428a6d35a8d88d79f7f1e3f02\
    03010001a31a301830090603551d1304\
    023000300b0603551d0f0404030205a0\
    300d06092a864886f70d01010b050003\
    81810085aad2a0e5b9276b908c65f73a\
    7267170618a54c5f8a7b337d2df7a594\
    365417f2eae8f8a58c8f8172f9319cf3\
    6b7fd6c55b80f21a03015156726096fd\
    335e5e67f2dbf102702e608ccae6bec1\
    fc63a42a99be5c3eb7107c3c54e9b9eb\
    2bd5203b1c3b84e0a8b2f759409ba3ea\
    c9d91d402dcc0cc8f8961229ac9187b4\
    2b4de100000f000084080400805a747c\
    5d88fa9bd2e55ab085a61015b7211f82\
    4cd484145ab3ff52f1fda8477b0b7abc\
    90db78e2d33a5c141a078653fa6bef78\
    0c5ea248eeaaa785c4f394cab6d30bbe\
    8d4859ee511f602957b15411ac027671\
    459e46445c9ea58c181e818e95b8c3fb\
    0bf3278409d3be152a3da5043e063dda\
    65cdf5aea20d53dfacd42f74f3140000\
    209b9b141d906337fbd2cbdce71df4de\
    da4ab42c309572cb7fffee5454b78f07\
    18\
    ";

/// The client's Finished: 36 octets.
pub const CLIENT_FINISHED: &str = "\
    14000020a8ec436d677634ae525ac1fc\
    ebe11a039ec17694fac6e98527b642f2\
    edd5ce61\
    ";

/// `Hash(ClientHello..ServerHello)`, which RFC 8448 publishes and stage 3a
/// took as an input.
pub const TRANSCRIPT_HELLO: &str =
    "860c06edc07858ee8e78f0e7428c58edd6b43f2ca3e6e95f02ed063cf0e1cad8";
/// `Hash(ClientHello..server Finished)`.
pub const TRANSCRIPT_SERVER_FINISHED: &str =
    "9608102a0f1ccc6db6250b7b7e417b1a000eaada3daae4777a7686c9ff83df13";
/// `Hash(ClientHello..client Finished)`.
pub const TRANSCRIPT_CLIENT_FINISHED: &str =
    "209145a96ee8e2a122ff810047cc952684658d6049e86429426db87c54ad143d";

/// The ECDHE shared secret, from stage 3a.
pub const SHARED_SECRET: &str = "8bd4054fb55b9d63fdfbacf9f04b9f0d35e6d63f537563efd46272900f89492d";
/// The server's Finished `verify_data`.
pub const SERVER_VERIFY_DATA: &str =
    "9b9b141d906337fbd2cbdce71df4deda4ab42c309572cb7fffee5454b78f0718";

/// Every message in the exchange, in order — the seed corpus a fuzzer wants.
pub fn all_messages() -> Vec<Vec<u8>> {
    vec![
        hex(CLIENT_HELLO),
        hex(SERVER_HELLO),
        hex(SERVER_FLIGHT),
        hex(CLIENT_FINISHED),
    ]
}
