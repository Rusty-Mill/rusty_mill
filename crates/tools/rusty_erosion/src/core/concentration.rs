//! CC concentration: the share of cyclomatic complexity that sits in
//! functions whose CC is strictly above [`THRESHOLD`].

use super::{FunctionMetric, Ratio};

/// Functions with CC strictly greater than this count as concentrated.
pub const THRESHOLD: u32 = 10;

/// CC in functions above [`THRESHOLD`] over the CC of all functions.
pub fn share(functions: &[FunctionMetric]) -> Ratio {
    let cc = |f: &FunctionMetric| u64::from(f.cc);
    Ratio {
        num: functions.iter().filter(|f| f.cc > THRESHOLD).map(cc).sum(),
        den: functions.iter().map(cc).sum(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(cc: u32) -> FunctionMetric {
        FunctionMetric {
            key: format!("f{cc}"),
            path: "a.rs".into(),
            line: 1,
            cc,
        }
    }

    #[test]
    fn counts_only_strictly_above_threshold() {
        assert_eq!(share(&[f(10), f(10)]), Ratio { num: 0, den: 20 });
        assert_eq!(share(&[f(11), f(9)]), Ratio { num: 11, den: 20 });
        assert_eq!(share(&[]), Ratio::default());
    }
}
