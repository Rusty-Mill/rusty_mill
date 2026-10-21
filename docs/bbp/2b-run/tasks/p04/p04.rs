use target::{join_record, split_record};
fn v(x: &[&str]) -> Option<Vec<String>> { Some(x.iter().map(|s| s.to_string()).collect()) }
#[test] fn splits_plain() { assert_eq!(split_record("a,b,c"), v(&["a", "b", "c"])); assert_eq!(split_record(""), v(&[""])); assert_eq!(split_record("a,"), v(&["a", ""])); }
#[test] fn splits_quoted() { assert_eq!(split_record("\"a,b\",c"), v(&["a,b", "c"])); assert_eq!(split_record("\"a\"\"b\""), v(&["a\"b"])); }
#[test] fn refuses_bad_quotes() { assert_eq!(split_record("\"a"), None); assert_eq!(split_record("a\"b"), None); assert_eq!(split_record("\"a\"b"), None); }
#[test] fn joins() { assert_eq!(join_record(&["a", "b"]), "a,b"); assert_eq!(join_record(&["a,b"]), "\"a,b\""); assert_eq!(join_record(&["a\"b"]), "\"a\"\"b\""); assert_eq!(join_record(&[" a"]), "\" a\""); }
#[test] fn round_trips() { let f = ["x", "", "a,b", "q\"q", " s ", "end"]; assert_eq!(split_record(&join_record(&f)), v(&f)); }
