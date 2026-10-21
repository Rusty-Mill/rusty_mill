use target::{from_roman, to_roman};
#[test] fn encodes() { assert_eq!(to_roman(1994).as_deref(), Some("MCMXCIV")); assert_eq!(to_roman(3999).as_deref(), Some("MMMCMXCIX")); assert_eq!(to_roman(4).as_deref(), Some("IV")); }
#[test] fn encode_range() { assert_eq!(to_roman(0), None); assert_eq!(to_roman(4000), None); }
#[test] fn decodes() { assert_eq!(from_roman("MCMXCIV"), Some(1994)); assert_eq!(from_roman("XL"), Some(40)); }
#[test] fn refuses_non_canonical() { for s in ["", "IIII", "VX", "IC", "mcm", "MMMM", "IIV"] { assert_eq!(from_roman(s), None, "{s}"); } }
#[test] fn round_trips() { for n in 1..=3999 { assert_eq!(from_roman(&to_roman(n).unwrap()), Some(n)); } }
