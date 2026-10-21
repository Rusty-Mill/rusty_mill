use target::normalize;
#[test] fn dot_segments() { assert_eq!(normalize("/a/./b/../c"), "/a/c"); assert_eq!(normalize("a/.."), "."); }
#[test] fn parent_above_root() { assert_eq!(normalize("/../a"), "/a"); assert_eq!(normalize("../a/.."), ".."); assert_eq!(normalize("../../x"), "../../x"); }
#[test] fn separators() { assert_eq!(normalize("a//b/"), "a/b/"); assert_eq!(normalize("//a"), "/a"); }
#[test] fn trailing_slash_after_dots() { assert_eq!(normalize("a/b/../"), "a/"); assert_eq!(normalize("./"), "."); assert_eq!(normalize("/a/../"), "/"); }
#[test] fn empty() { assert_eq!(normalize(""), "."); assert_eq!(normalize("/"), "/"); }
