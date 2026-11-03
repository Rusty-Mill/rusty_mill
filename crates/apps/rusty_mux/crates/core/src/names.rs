//! Generated session names: an adjective and a noun, such as `quiet-harbor`.
//!
//! A session created without a name gets one, unlike every other session's,
//! so a person or an agent can tell sessions apart and name one in a
//! command. [`is_generated`] tells whether a name has the shape of a
//! generated one. It reads only the spelling: a person may choose such a
//! name too (`create-session --name quiet-harbor`), so it is no proof that
//! nobody named a session.

/// Short, plain words in lowercase ASCII, without dashes.
const ADJECTIVES: [&str; 64] = [
    "amber", "ancient", "autumn", "bold", "brave", "bright", "brisk", "calm", "clever", "cosy",
    "crisp", "dapper", "dawn", "deep", "eager", "early", "fair", "fancy", "fond", "frosty",
    "gentle", "glad", "golden", "grand", "green", "happy", "hidden", "humble", "jolly", "keen",
    "kind", "late", "lively", "lucky", "mellow", "merry", "misty", "modest", "nimble", "noble",
    "patient", "plain", "polite", "proud", "quick", "quiet", "rapid", "ready", "rosy", "royal",
    "rustic", "silent", "silver", "steady", "still", "sunny", "swift", "tidy", "velvet", "vivid",
    "warm", "wild", "wise", "young",
];

const NOUNS: [&str; 64] = [
    "acorn", "anchor", "aspen", "badger", "beacon", "birch", "brook", "canyon", "cedar", "comet",
    "coral", "crane", "delta", "dune", "falcon", "fern", "field", "finch", "fjord", "forest",
    "fox", "garden", "glacier", "grove", "harbor", "hare", "hawk", "heron", "hill", "island",
    "lagoon", "lake", "lantern", "maple", "meadow", "mesa", "moon", "moss", "oak", "orchard",
    "otter", "owl", "pebble", "pine", "plover", "pond", "prairie", "quarry", "raven", "reef",
    "ridge", "river", "robin", "sparrow", "spruce", "stone", "summit", "thistle", "tide", "valley",
    "willow", "wren", "yarrow", "zephyr",
];

/// How many distinct names there are before a number is added.
const PAIRS: usize = ADJECTIVES.len() * NOUNS.len();

/// The `index`th adjective-noun pair, `index < PAIRS`.
fn pair(index: usize) -> String {
    let adjective = ADJECTIVES[index / NOUNS.len()];
    let noun = NOUNS[index % NOUNS.len()];
    format!("{adjective}-{noun}")
}

/// A generated name that `taken` refuses, chosen from `seed`: the pair the
/// seed picks, else the next free pair after it, else that first pair with
/// the smallest free number from 2, as `quiet-harbor-2`.
pub(crate) fn generate(seed: u64, taken: impl Fn(&str) -> bool) -> String {
    // A u64 modulo 4096 never truncates.
    let start = (seed % PAIRS as u64) as usize;
    if let Some(free) = (0..PAIRS)
        .map(|step| pair((start + step) % PAIRS))
        .find(|name| !taken(name))
    {
        return free;
    }
    let base = pair(start);
    (2u64..)
        .map(|n| format!("{base}-{n}"))
        .find(|name| !taken(name))
        .unwrap_or(base)
}

/// Whether `name` has the form a generated name has: one of the adjectives,
/// a dash, one of the nouns, and optionally a dash and a number from 2
/// without leading zeros.
pub fn is_generated(name: &str) -> bool {
    let mut parts = name.splitn(3, '-');
    let (Some(adjective), Some(noun)) = (parts.next(), parts.next()) else {
        return false;
    };
    if !ADJECTIVES.contains(&adjective) || !NOUNS.contains(&noun) {
        return false;
    }
    match parts.next() {
        None => true,
        Some(number) => {
            !number.starts_with('0')
                && number.bytes().all(|b| b.is_ascii_digit())
                && number.parse::<u64>().is_ok_and(|n| n >= 2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_are_unique_plain_and_dash_free() {
        for words in [&ADJECTIVES[..], &NOUNS[..]] {
            for (i, word) in words.iter().enumerate() {
                assert!(!word.is_empty() && word.bytes().all(|b| b.is_ascii_lowercase()));
                assert!(!words[..i].contains(word), "{word} twice");
            }
        }
    }

    #[test]
    fn the_seed_picks_the_pair() {
        assert_eq!(generate(0, |_| false), "amber-acorn");
        assert_eq!(generate(65, |_| false), "ancient-anchor");
        // Wraps rather than overflowing.
        assert_eq!(generate(PAIRS as u64, |_| false), "amber-acorn");
        assert_eq!(generate(u64::MAX, |_| false), "young-zephyr");
    }

    #[test]
    fn a_taken_pair_moves_on_to_the_next_free_one() {
        let name = generate(0, |n| n == "amber-acorn" || n == "amber-anchor");
        assert_eq!(name, "amber-aspen");
        // Past the last pair it wraps to the first.
        assert_eq!(generate(u64::MAX, |n| n == "young-zephyr"), "amber-acorn");
    }

    #[test]
    fn with_every_pair_taken_a_number_is_added() {
        let name = generate(1, |n| !n.ends_with("-3"));
        assert_eq!(name, "amber-anchor-3");
    }

    #[test]
    fn every_generated_name_is_recognised() {
        assert!((0..PAIRS).all(|i| is_generated(&pair(i))));
        assert!(is_generated("quiet-harbor-2"));
        assert!(is_generated("quiet-harbor-10"));
    }

    #[test]
    fn chosen_names_are_not_generated() {
        for name in [
            "",
            "main",
            "quiet",
            "quiet-",
            "quiet-harbor-",
            "quiet-harbor-1",
            "quiet-harbor-0",
            "quiet-harbor-02",
            "quiet-harbor-x",
            "quiet-harbor-2-2",
            "quiet-harbor-+2",
            "harbor-quiet",
            "Quiet-harbor",
            "quiet-harbor-99999999999999999999999",
            "fix-login-bug",
        ] {
            assert!(!is_generated(name), "{name}");
        }
    }
}
