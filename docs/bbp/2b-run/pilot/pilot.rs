use target::{initials, title_case};
#[test] fn titles() { assert_eq!(title_case("hello  wORLD"), "Hello World"); assert_eq!(title_case(""), ""); }
#[test] fn initial_letters() { assert_eq!(initials("ada  lovelace"), "AL"); assert_eq!(initials("  "), ""); }
