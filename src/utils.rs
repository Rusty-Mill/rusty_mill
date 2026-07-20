//! Port of the parts of `src/utils` needed so far.
//!
//! The Go package is a grab-bag (hashing, filesystem walking, IP discovery,
//! progress helpers); pieces are ported as the modules that need them arrive.

use crate::mnemonicode;
use rand::Rng;

/// Matches Go's `utils.NbPinNumbers`.
pub const NB_PIN_NUMBERS: usize = 4;
/// Matches Go's `utils.NbBytesWords`.
pub const NB_BYTES_WORDS: usize = 4;

/// Random numeric pin, mirroring `utils.GenerateRandomPin`
/// (Go draws each digit from `[0, 9)`).
pub fn generate_random_pin() -> String {
    let mut rng = rand::thread_rng();
    (0..NB_PIN_NUMBERS)
        .map(|_| char::from(b'0' + rng.gen_range(0..9u8)))
        .collect()
}

/// Random code phrase like `1234-quiet-lion-daisy`, mirroring
/// `utils.GetRandomName`.
pub fn get_random_name() -> String {
    let mut bs = [0u8; NB_BYTES_WORDS];
    rand::thread_rng().fill(&mut bs);
    let words = mnemonicode::encode_word_list(&bs);
    format!("{}-{}", generate_random_pin(), words.join("-"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pin_shape() {
        let pin = generate_random_pin();
        assert_eq!(pin.len(), NB_PIN_NUMBERS);
        assert!(pin.chars().all(|c| c.is_ascii_digit()));
    }

    #[test]
    fn name_shape() {
        let name = get_random_name();
        let parts: Vec<&str> = name.split('-').collect();
        assert_eq!(parts.len(), 1 + mnemonicode::words_required(NB_BYTES_WORDS));
    }
}
