//! Pure domain core: no I/O and no parser. Adapters turn source files into
//! [`FileMeasure`]s; [`Report::build`] turns those into the erosion numbers.

pub mod concentration;
pub mod duplication;

use std::collections::HashMap;

/// Basis points in one whole: 10 000 bp = 100 %.
pub const BP: u64 = 10_000;

/// A language the core measures. Duplication never matches across languages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Lang {
    Rust,
}

/// One normalized token. Adapters map identifiers and literals to fixed
/// classes and keep keyword and punctuation text, so equal tokens compare
/// equal by value.
pub type NormTok = &'static str;

/// One function: a stable key for matching baseline and candidate, and its
/// cyclomatic complexity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionMetric {
    /// `path::Type::name`, with `#n` appended to the n-th repeat in a file.
    pub key: String,
    pub path: String,
    /// 1-based line of the function's first token.
    pub line: u32,
    pub cc: u32,
}

/// What an adapter extracts from one measured file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileMeasure {
    pub path: String,
    pub lang: Lang,
    pub functions: Vec<FunctionMetric>,
    pub tokens: Vec<NormTok>,
    /// Non-blank, non-comment lines outside skipped test code.
    pub loc: u64,
    /// The parser recovered from syntax errors; the file is still measured.
    pub parse_error: bool,
}

/// An exact share, `num / den`. Deltas are taken from these, not from
/// rounded basis points, so a small change in a large tree is not lost.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Ratio {
    pub num: u64,
    pub den: u64,
}

impl Ratio {
    /// The share in basis points, rounded down; 0 when `den` is 0.
    pub fn bp(self) -> u32 {
        match self.den {
            0 => 0,
            den => (u128::from(self.num) * u128::from(BP) / u128::from(den)) as u32,
        }
    }

    /// `other - self` in basis points, exact before truncating toward zero.
    pub fn delta_bp(self, other: Ratio) -> i32 {
        let (a, b) = (self.exact(), other.exact());
        let num = b.0 * a.1 - a.0 * b.1;
        (num * i128::from(BP) / (a.1 * b.1)) as i32
    }

    /// The ratio with an empty denominator read as 0 / 1.
    fn exact(self) -> (i128, i128) {
        match self.den {
            0 => (0, 1),
            den => (i128::from(self.num), i128::from(den)),
        }
    }
}

/// The erosion numbers for a set of files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    /// CC in functions with CC above [`concentration::THRESHOLD`], over all CC.
    pub concentration: Ratio,
    /// Tokens inside repeated normalized windows, over all tokens.
    pub duplication: Ratio,
    pub total_cc: u64,
    pub loc: u64,
    /// Every measured function, sorted by `(path, line, key)`.
    pub functions: Vec<FunctionMetric>,
    /// Code files in a language the core cannot measure, sorted.
    pub unmeasured: Vec<String>,
    /// Measured files the parser had to recover in, sorted.
    pub parse_errors: Vec<String>,
}

impl Report {
    /// Builds the report. The result does not depend on input order.
    pub fn build(mut files: Vec<FileMeasure>, mut unmeasured: Vec<String>) -> Report {
        files.sort_by(|a, b| a.path.cmp(&b.path));
        unmeasured.sort();
        let mut functions: Vec<FunctionMetric> = files
            .iter()
            .flat_map(|f| f.functions.iter().cloned())
            .collect();
        functions.sort_by(|a, b| (&a.path, a.line, &a.key).cmp(&(&b.path, b.line, &b.key)));
        let parse_errors = files
            .iter()
            .filter(|f| f.parse_error)
            .map(|f| f.path.clone())
            .collect();
        Report {
            concentration: concentration::share(&functions),
            duplication: duplication::share(&files),
            total_cc: functions.iter().map(|f| u64::from(f.cc)).sum(),
            loc: files.iter().map(|f| f.loc).sum(),
            functions,
            unmeasured,
            parse_errors,
        }
    }

    /// The `n` most complex functions: CC descending, then key, then line.
    pub fn worst(&self, n: usize) -> Vec<&FunctionMetric> {
        let mut all: Vec<&FunctionMetric> = self.functions.iter().collect();
        all.sort_by(|a, b| {
            b.cc.cmp(&a.cc)
                .then_with(|| (&a.key, a.line).cmp(&(&b.key, b.line)))
        });
        all.truncate(n);
        all
    }

    /// The change from `self` (the baseline) to `candidate`.
    pub fn delta(&self, candidate: &Report) -> Delta {
        Delta {
            concentration_bp: self.concentration.delta_bp(candidate.concentration),
            duplication_bp: self.duplication.delta_bp(candidate.duplication),
            total_cc: diff(self.total_cc, candidate.total_cc),
            loc: diff(self.loc, candidate.loc),
            functions: diff(
                self.functions.len() as u64,
                candidate.functions.len() as u64,
            ),
        }
    }

    /// The `n` functions whose CC rose most from `self` (the baseline) to
    /// `candidate`, matched by key; a new function counts from 0. Ties go by
    /// key. Functions whose CC did not rise are left out.
    pub fn worst_affected(&self, candidate: &Report, n: usize) -> Vec<Affected> {
        let base: HashMap<&str, u32> = self
            .functions
            .iter()
            .map(|f| (f.key.as_str(), f.cc))
            .collect();
        let mut out: Vec<Affected> = candidate
            .functions
            .iter()
            .map(|f| Affected {
                key: f.key.clone(),
                base_cc: base.get(f.key.as_str()).copied().unwrap_or(0),
                cc: f.cc,
            })
            .filter(|a| a.cc > a.base_cc)
            .collect();
        out.sort_by(|a, b| {
            (b.cc - b.base_cc)
                .cmp(&(a.cc - a.base_cc))
                .then_with(|| a.key.cmp(&b.key))
        });
        out.truncate(n);
        out
    }
}

/// A function whose CC rose between baseline and candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Affected {
    pub key: String,
    /// 0 when the function is new.
    pub base_cc: u32,
    pub cc: u32,
}

/// Signed change from a baseline to a candidate. Positive is worse for the
/// two shares. The counts let calibration spot gaming, such as function count
/// or total CC rising while concentration falls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Delta {
    pub concentration_bp: i32,
    pub duplication_bp: i32,
    pub total_cc: i64,
    pub loc: i64,
    pub functions: i64,
}

fn diff(base: u64, candidate: u64) -> i64 {
    candidate as i64 - base as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bp_floors_and_empty_is_zero() {
        assert_eq!(Ratio { num: 1, den: 3 }.bp(), 3333);
        assert_eq!(Ratio::default().bp(), 0);
        assert_eq!(Ratio { num: 5, den: 5 }.bp(), BP as u32);
    }

    #[test]
    fn delta_is_exact_not_from_rounded_bp() {
        // Both round to 0 bp, but the change is still 0.99 bp -> truncates to 0,
        // while 1 -> 3 in 10 000 is exactly +2 bp.
        let base = Ratio {
            num: 1,
            den: 10_000,
        };
        assert_eq!(
            base.delta_bp(Ratio {
                num: 3,
                den: 10_000
            }),
            2
        );
        assert_eq!(
            Ratio { num: 3, den: 4 }.delta_bp(Ratio { num: 1, den: 4 }),
            -5000
        );
        assert_eq!(Ratio::default().delta_bp(Ratio { num: 1, den: 2 }), 5000);
    }

    fn f(key: &str, cc: u32) -> FunctionMetric {
        FunctionMetric {
            key: key.into(),
            path: "a.rs".into(),
            line: 1,
            cc,
        }
    }

    #[test]
    fn worst_affected_ranks_cc_rises_by_key() {
        let base = Report::build(vec![], vec![]);
        let mut base_fns = base.clone();
        base_fns.functions = vec![f("a", 5), f("b", 2), f("c", 9)];
        let mut cand = base.clone();
        cand.functions = vec![f("a", 8), f("b", 2), f("c", 4), f("d", 3), f("e", 4)];
        let got: Vec<(String, u32, u32)> = base_fns
            .worst_affected(&cand, 3)
            .into_iter()
            .map(|a| (a.key, a.base_cc, a.cc))
            .collect();
        assert_eq!(
            got,
            [("e".into(), 0, 4), ("a".into(), 5, 8), ("d".into(), 0, 3)]
        );
    }
}
