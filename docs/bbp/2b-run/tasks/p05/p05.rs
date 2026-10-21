use target::{count_words, wrap};
#[test] fn wraps_greedily() { assert_eq!(wrap("the quick brown fox", 10), vec!["the quick", "brown fox"]); }
#[test] fn long_word_alone() { assert_eq!(wrap("a verylongword b", 5), vec!["a", "verylongword", "b"]); }
#[test] fn whitespace_runs_collapse() { assert_eq!(wrap("  a \n\t b  ", 80), vec!["a b"]); }
#[test] fn no_words_no_lines() { assert!(wrap("   ", 10).is_empty()); assert!(wrap("", 10).is_empty()); }
#[test] fn counts() { assert_eq!(count_words("one two\tthree\nfour"), 4); assert_eq!(count_words("  "), 0); assert_eq!(count_words("caf\u{e9}-au-lait x"), 2); }
