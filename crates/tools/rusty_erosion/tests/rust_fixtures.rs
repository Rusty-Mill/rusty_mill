//! End-to-end checks of the Rust adapter and the core on small fixtures.

use rusty_erosion::{Report, measure};

fn report(files: &[(&str, &str)]) -> Report {
    match measure(files.iter().copied()) {
        Ok(r) => r,
        Err(e) => panic!("measure failed: {e}"),
    }
}

fn cc_of(r: &Report, key: &str) -> u32 {
    match r.functions.iter().find(|f| f.key == key) {
        Some(f) => f.cc,
        None => panic!("no function {key} in {:?}", r.functions),
    }
}

const BRANCHY: &str = r#"
struct S;
impl<T> Show for Wrap<T> {
    fn fmt(&self) -> u8 {
        if a && b || c { 1 } else if let Some(x) = y { 2 } else { 3 }
    }
}
impl S {
    fn arms(&self, x: u8) -> u8 {
        match x { 1 => 2, _ if x > 9 => 3, _ => 4 }
    }
    fn loops(&self) {
        while go() { for _ in 0..3 { loop { break; } } }
        let f = |v: u8| if v > 0 { v } else { 0 };
        let r = maybe()?;
        fn inner() { if z {} }
    }
}
fn plain() {}
"#;

#[test]
fn cyclomatic_complexity_per_function() {
    let r = report(&[("src/s.rs", BRANCHY)]);
    // if, &&, ||, else if.
    assert_eq!(cc_of(&r, "src/s.rs::<Wrap as Show>::fmt"), 5);
    // two extra arms and one guard.
    assert_eq!(cc_of(&r, "src/s.rs::S::arms"), 4);
    // while, for, the closure's if; loop and ? add nothing; inner is its own.
    assert_eq!(cc_of(&r, "src/s.rs::S::loops"), 4);
    assert_eq!(cc_of(&r, "src/s.rs::S::loops::inner"), 2);
    assert_eq!(cc_of(&r, "src/s.rs::plain"), 1);
    assert_eq!(r.total_cc, 16);
    assert_eq!(r.functions.len(), 5);
}

fn ladder(name: &str, rungs: usize) -> String {
    let body: String = (0..rungs)
        .map(|i| format!("if x == {i} {{ return {i}; }}\n"))
        .collect();
    format!("fn {name}(x: u8) -> u8 {{\n{body}0\n}}\n")
}

#[test]
fn concentration_counts_functions_above_ten() {
    let src = format!("{}{}", ladder("big", 10), ladder("small", 9));
    let r = report(&[("src/l.rs", &src)]);
    assert_eq!(cc_of(&r, "src/l.rs::big"), 11);
    assert_eq!((r.concentration.num, r.concentration.den), (11, 21));
    assert_eq!(r.concentration.bp(), 5238);
    assert_eq!(r.worst(1)[0].key, "src/l.rs::big");
}

const ORIGINAL: &str = r#"
pub fn total(items: &[Item], rate: u32) -> u32 {
    let mut sum = 0;
    for item in items.iter() {
        let price = item.price * item.count;
        let tax = price * rate / 100;
        sum += price + tax;
    }
    sum
}
"#;

const RENAMED: &str = r#"
// A pasted copy with every name and literal changed.
pub fn grand(goods: &[Thing], pct: u32) -> u32 {
    let mut acc = 7;
    for g in goods.iter() {
        let cost = g.cost * g.qty;
        let duty = cost * pct / 250;
        acc += cost + duty;
    }
    acc
}
"#;

#[test]
fn a_renamed_clone_is_still_detected() {
    let alone = report(&[("src/a.rs", ORIGINAL)]);
    assert_eq!(alone.duplication.num, 0);
    let pasted = report(&[("src/a.rs", ORIGINAL), ("src/b.rs", RENAMED)]);
    assert_eq!(pasted.duplication.num, pasted.duplication.den);
    let d = alone.delta(&pasted);
    assert_eq!(d.duplication_bp, 10_000);
    assert_eq!(d.functions, 1);
}

#[test]
fn test_code_comments_and_non_code_are_skipped() {
    let with_tests = format!(
        "{ORIGINAL}\n/// doc\n#[cfg(test)]\nmod tests {{ {RENAMED} }}\n#[test]\nfn t() {{ if a {{}} }}\n"
    );
    let r = report(&[
        ("src/a.rs", &with_tests),
        ("crate/tests/it.rs", RENAMED),
        ("src/tests.rs", RENAMED),
        ("README.md", "# hi"),
        ("tools/gen.py", "print(1)"),
    ]);
    let base = report(&[("src/a.rs", ORIGINAL)]);
    assert_eq!(r.duplication, base.duplication);
    assert_eq!(r.functions, base.functions);
    assert_eq!(r.loc, base.loc);
    assert_eq!(r.unmeasured, vec!["tools/gen.py".to_owned()]);
    let whole = report(&[("src/x.rs", "#![cfg(test)]\nfn a() { if b {} }")]);
    assert!(whole.functions.is_empty());
}

#[test]
fn loc_counts_code_lines_only() {
    let r = report(&[("src/a.rs", "// c\n\nfn a() {\n    /* b */\n    x();\n}\n")]);
    assert_eq!(r.loc, 3);
}

#[test]
fn repeated_keys_are_disambiguated() {
    let src = "#[cfg(unix)]\nfn a() {}\n#[cfg(windows)]\nfn a() {}\n";
    let r = report(&[("src/a.rs", src)]);
    let keys: Vec<&str> = r.functions.iter().map(|f| f.key.as_str()).collect();
    assert_eq!(keys, ["src/a.rs::a", "src/a.rs::a#2"]);
}

#[test]
fn parse_errors_are_measured_and_listed() {
    let r = report(&[
        ("src/bad.rs", "fn a() { if x { }"),
        ("src/ok.rs", "fn b() {}"),
    ]);
    assert_eq!(r.parse_errors, vec!["src/bad.rs".to_owned()]);
    assert!(r.loc > 1);
}

#[test]
fn output_is_deterministic_and_order_independent() {
    let big = ladder("big", 12);
    let files = [
        ("src/a.rs", ORIGINAL),
        ("src/b.rs", RENAMED),
        ("src/c.rs", big.as_str()),
        ("x.go", ""),
    ];
    let first = report(&files);
    let mut reversed = files;
    reversed.reverse();
    for _ in 0..3 {
        assert_eq!(report(&files), first);
        assert_eq!(report(&reversed), first);
    }
}
