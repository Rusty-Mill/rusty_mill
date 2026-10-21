use target::{format_duration, parse_duration};
#[test] fn parses_parts() { assert_eq!(parse_duration("1h30m"), Some(5400)); assert_eq!(parse_duration("2m5s"), Some(125)); assert_eq!(parse_duration("45s"), Some(45)); }
#[test] fn refuses_bad_order_and_repeats() { assert_eq!(parse_duration("5m1h"), None); assert_eq!(parse_duration("1h1h"), None); }
#[test] fn refuses_empty_and_bare() { assert_eq!(parse_duration(""), None); assert_eq!(parse_duration("10"), None); assert_eq!(parse_duration("1h 2m"), None); }
#[test] fn refuses_overflow() { assert_eq!(parse_duration("99999999999999999999s"), None); assert_eq!(parse_duration("9999999999999999h"), None); }
#[test] fn formats() { assert_eq!(format_duration(5400), "1h30m"); assert_eq!(format_duration(3601), "1h1s"); assert_eq!(format_duration(45), "45s"); }
#[test] fn formats_zero() { assert_eq!(format_duration(0), "0s"); }
