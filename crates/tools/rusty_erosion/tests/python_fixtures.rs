//! End-to-end checks of the Python adapter (card #705) on small fixtures.

use rusty_erosion::{Lang, Report, measure, measure_in};

fn report(files: &[(&str, &str)]) -> Report {
    match measure_in(&[Lang::Python], files.iter().copied()) {
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
import os

if os.name == "nt":
    SEP = "\\"

class Shape:
    def area(self, x):
        if x > 0 and self.ok or x < -9:
            return 1
        elif x == 0:
            return 2
        else:
            return 3 if x else 4

    def loops(self, xs):
        for x in xs:
            while x:
                x -= 1
        else:
            pass
        ys = [y for y in xs if y]
        f = lambda v: v if v else 0
        def inner():
            assert xs
        return ys

def guarded(path):
    try:
        with open(path) as fh:
            return fh.read()
    except OSError:
        return None
    finally:
        pass

def dispatch(cmd):
    match cmd:
        case "a":
            return 1
        case "b":
            return 2
        case _:
            return 3

def plain():
    pass
"#;

#[test]
fn cyclomatic_complexity_per_function() {
    let r = report(&[("app/shape.py", BRANCHY)]);
    // if, and, or, elif, conditional expression; else adds nothing.
    assert_eq!(cc_of(&r, "app/shape.py::Shape::area"), 6);
    // for, while, loop else, comprehension for and if, the lambda's
    // conditional; inner is its own unit.
    assert_eq!(cc_of(&r, "app/shape.py::Shape::loops"), 7);
    assert_eq!(cc_of(&r, "app/shape.py::Shape::loops::inner"), 2);
    // with, except; try and finally add nothing.
    assert_eq!(cc_of(&r, "app/shape.py::guarded"), 3);
    // three cases, two extra.
    assert_eq!(cc_of(&r, "app/shape.py::dispatch"), 3);
    assert_eq!(cc_of(&r, "app/shape.py::plain"), 1);
    // Module-level `if` adds no CC.
    assert_eq!(r.total_cc, 22);
    assert_eq!(r.functions.len(), 6);
}

fn ladder(name: &str, rungs: usize) -> String {
    let body: String = (0..rungs)
        .map(|i| format!("    if x == {i}:\n        return {i}\n"))
        .collect();
    format!("def {name}(x):\n{body}    return 0\n")
}

#[test]
fn concentration_counts_functions_above_ten() {
    let src = format!("{}{}", ladder("big", 10), ladder("small", 9));
    let r = report(&[("app/l.py", &src)]);
    assert_eq!((r.concentration.num, r.concentration.den), (11, 21));
}

const ORIGINAL: &str = r#"
def total(items, rate):
    acc = 0
    for item in items:
        if item.price > 100 and item.taxed:
            acc += item.price * rate // 100 + item.fee
        else:
            acc += item.price + item.fee - item.discount
    return acc
"#;

const RENAMED: &str = r#"
def grand(goods, pct):
    s = 0
    for g in goods:
        if g.cost > 7 and g.levied:
            s += g.cost * pct // 3 + g.charge
        else:
            s += g.cost + g.charge - g.rebate
    return s
"#;

#[test]
fn a_renamed_clone_is_still_detected() {
    let alone = report(&[("app/a.py", ORIGINAL)]);
    assert_eq!(alone.duplication.num, 0);
    let pasted = report(&[("app/a.py", ORIGINAL), ("app/b.py", RENAMED)]);
    assert!(pasted.duplication.den >= 100);
    assert_eq!(pasted.duplication.num, pasted.duplication.den);
}

#[test]
fn comments_test_files_and_python_in_the_gate_are_skipped() {
    let with_comments = format!("# header\n{ORIGINAL}    # trailing\n");
    let a = report(&[("app/a.py", ORIGINAL)]);
    let b = report(&[("app/a.py", &with_comments)]);
    assert_eq!(a.duplication, b.duplication);
    assert_eq!(a.total_cc, b.total_cc);

    let tests = report(&[
        ("app/a.py", ORIGINAL),
        ("app/test_a.py", RENAMED),
        ("tests/b.py", RENAMED),
    ]);
    assert_eq!(tests.duplication.num, 0);

    let gate = match measure([("app/a.py", ORIGINAL)]) {
        Ok(r) => r,
        Err(e) => panic!("measure failed: {e}"),
    };
    assert_eq!(gate.total_cc, 0);
    assert_eq!(gate.unmeasured, vec!["app/a.py".to_owned()]);
}

#[test]
fn loc_counts_code_lines_only() {
    let r = report(&[("app/a.py", "# c\n\ndef a():\n    x()\n\n    y()\n")]);
    assert_eq!(r.loc, 3);
}

#[test]
fn rust_and_python_never_match_each_other() {
    let rust = "fn f() { a(); }";
    let r = match measure_in(
        &[Lang::Rust, Lang::Python],
        [("app/a.py", ORIGINAL), ("src/a.rs", rust)],
    ) {
        Ok(r) => r,
        Err(e) => panic!("measure failed: {e}"),
    };
    assert_eq!(r.functions.len(), 2);
    assert!(r.unmeasured.is_empty());
}

#[test]
fn output_is_deterministic_and_order_independent() {
    let files = [
        ("app/a.py", ORIGINAL),
        ("app/b.py", RENAMED),
        ("app/s.py", BRANCHY),
    ];
    let mut reversed = files;
    reversed.reverse();
    assert_eq!(report(&files), report(&reversed));
}
