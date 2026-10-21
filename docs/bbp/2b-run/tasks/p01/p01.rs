use target::{slug, truncate_slug};
#[test] fn slug_lowercases_and_joins() { assert_eq!(slug("Hello, World!"), "hello-world"); }
#[test] fn slug_collapses_and_trims() { assert_eq!(slug("--a  --  b9--"), "a-b9"); }
#[test] fn slug_empty() { assert_eq!(slug("!!"), ""); }
#[test] fn short_is_unchanged() { assert_eq!(truncate_slug("ab-cd", 5), "ab-cd"); }
#[test] fn cuts_at_word_boundary() { assert_eq!(truncate_slug("hello-big-world", 9), "hello-big"); assert_eq!(truncate_slug("hello-big-world", 8), "hello"); }
#[test] fn hard_cuts_a_long_first_word() { assert_eq!(truncate_slug("abcdefghij", 4), "abcd"); }
#[test] fn zero_is_empty() { assert_eq!(truncate_slug("ab-cd", 0), ""); }
