//! LEL tab per-claim spend meter (card #1416, slice 5).
//!
//! Pure view model over the Mill Board `lel_spend` reply, `[{id, spent,
//! budget}]`, no UI and no I/O. Per-claim `spent` and `budget` only: the lens
//! list and spend share wait on an orch-side read (LEL Q42). State is a word,
//! never colour alone (D26); over budget is a word too, not a new colour (D54).

use super::lel::Error;
use rusty_json::Value;

/// Where a claim stands against its budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum State {
    /// Nothing is budgeted for the claim.
    Unbudgeted,
    /// Spent is within the budget.
    Within,
    /// Spent has passed the budget.
    Over,
}

/// One claim's meter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Meter {
    /// The claim message id.
    pub id: u64,
    /// Budget units spent.
    pub spent: u64,
    /// Budget units available.
    pub budget: u64,
}

impl Meter {
    /// The state word's source.
    pub fn state(&self) -> State {
        match (self.budget, self.spent) {
            (0, _) => State::Unbudgeted,
            (b, s) if s > b => State::Over,
            _ => State::Within,
        }
    }

    /// Filled share of the bar, 0 to 100; an unbudgeted claim has none.
    pub fn fill_percent(&self) -> u8 {
        if self.budget == 0 {
            return 0;
        }
        // Widened so a huge `spent` cannot overflow.
        let p = (u128::from(self.spent) * 100 / u128::from(self.budget)).min(100);
        u8::try_from(p).unwrap_or(100)
    }

    /// The meter's label, always words and numbers.
    pub fn label(&self) -> String {
        match self.state() {
            State::Unbudgeted => format!("{} spent, no budget", self.spent),
            State::Within => format!("{} of {} spent", self.spent, self.budget),
            State::Over => format!("{} of {} spent, over budget", self.spent, self.budget),
        }
    }
}

/// Parses a `lel_spend` reply. An empty list is no spend, not an error.
///
/// # Errors
/// [`Error`] on text that is not JSON, not a list, or an entry missing a
/// numeric `id`, `spent` or `budget`. One bad entry refuses the read: a
/// half-shown meter could hide an overspend.
pub(crate) fn parse(text: &str) -> Result<Vec<Meter>, Error> {
    let value = Value::parse(text).map_err(|e| Error::Json(e.to_string()))?;
    let items = value.as_array().ok_or(Error::NotAnArray)?;
    items
        .iter()
        .enumerate()
        .map(|(index, v)| {
            let n = |name: &'static str| {
                v.get(name)
                    .and_then(Value::as_u64)
                    .ok_or(Error::Field { index, name })
            };
            Ok(Meter {
                id: n("id")?,
                spent: n("spent")?,
                budget: n("budget")?,
            })
        })
        .collect()
}

/// The meter for claim `id`, if the reply had one.
pub(crate) fn find(meters: &[Meter], id: u64) -> Option<&Meter> {
    meters.iter().find(|m| m.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(spent: u64, budget: u64) -> Meter {
        Meter {
            id: 1,
            spent,
            budget,
        }
    }

    #[test]
    fn reads_the_reply() {
        let got =
            parse(r#"[{"id":12,"spent":30,"budget":100},{"id":13,"spent":0,"budget":0}]"#).unwrap();
        assert_eq!(got, [m(30, 100).with_id(12), m(0, 0).with_id(13)]);
        assert_eq!(find(&got, 13).map(|x| x.spent), Some(0));
        assert!(find(&got, 99).is_none());
    }

    impl Meter {
        fn with_id(mut self, id: u64) -> Self {
            self.id = id;
            self
        }
    }

    #[test]
    fn an_empty_reply_is_no_spend() {
        assert_eq!(parse("[]"), Ok(vec![]));
    }

    #[test]
    fn states_are_words() {
        assert_eq!(m(30, 100).label(), "30 of 100 spent");
        assert_eq!(m(100, 100).state(), State::Within);
        assert_eq!(m(101, 100).label(), "101 of 100 spent, over budget");
        assert_eq!(m(5, 0).label(), "5 spent, no budget");
    }

    #[test]
    fn the_bar_never_passes_full_or_overflows() {
        assert_eq!(m(0, 100).fill_percent(), 0);
        assert_eq!(m(50, 100).fill_percent(), 50);
        assert_eq!(m(500, 100).fill_percent(), 100);
        assert_eq!(m(u64::MAX, 1).fill_percent(), 100);
        assert_eq!(m(7, 0).fill_percent(), 0);
    }

    #[test]
    fn bad_input_is_named() {
        assert!(matches!(parse("nope"), Err(Error::Json(_))));
        assert_eq!(parse("{}"), Err(Error::NotAnArray));
        assert_eq!(
            parse(r#"[{"id":1,"spent":1,"budget":2},{"id":2,"spent":"x","budget":1}]"#),
            Err(Error::Field {
                index: 1,
                name: "spent"
            })
        );
        assert_eq!(
            parse(r#"[{"spent":1,"budget":1}]"#),
            Err(Error::Field {
                index: 0,
                name: "id"
            })
        );
    }
}
