//! Base64 payloads in terminal escapes (Kitty graphics, iTerm files, OSC 52):
//! the codec is `rusty_base64`'s; this only fixes the leniency those senders
//! need (line-wrapped input, padding that ends the data).

/// Decode standard base64, skipping ASCII whitespace and stopping at the first
/// `=`. Returns `None` on a byte outside the alphabet.
pub(crate) fn decode(input: &[u8]) -> Option<Vec<u8>> {
    rusty_base64::decode_standard_lenient(input).ok()
}
