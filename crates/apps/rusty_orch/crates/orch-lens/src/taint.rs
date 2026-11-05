//! Taint (#598 gap 2): what a lens saw marks what it posts.
//!
//! A post's class is the highest of its claim's domain class and the
//! highest class in the context the lens was shown, so a local lens cleared
//! for `sensitive` that saw sensitive input cannot launder it into an
//! `internal` post a cloud lens then reads. Taint never goes down. The
//! runner raises each post's taint after posting it, keeps the table in
//! orch-store, and applies it to every card it builds.

use std::collections::BTreeMap;

use rusty_bbp::MsgId;

use crate::card::Card;
use crate::class::{admit, DataClass};
use crate::context::Parts;

/// The highest class `parts` can show a provider cleared to `clearance`:
/// the card and every admitted evidence item and line. Counted whether or
/// not a line fits the window, so it may overstate, never understate.
pub fn of_context(parts: &Parts<'_>, clearance: DataClass) -> DataClass {
    let card = parts.card;
    let items = card.evidence.iter().map(|i| i.class);
    let lines = parts.lines.iter().map(|l| l.class);
    std::iter::once(card.class)
        .chain(items)
        .chain(lines)
        .filter(|c| admit(*c, clearance))
        .max()
        .unwrap_or(card.class)
}

/// Each tainted post's class, by message id.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Taints(BTreeMap<MsgId, DataClass>);

impl Taints {
    /// The table from stored `(message id, class)` pairs; the highest wins.
    pub fn from_entries(entries: impl IntoIterator<Item = (u64, DataClass)>) -> Self {
        let mut t = Self::default();
        for (msg, class) in entries {
            t.raise(MsgId(msg), class);
        }
        t
    }

    /// Every entry as `(message id, class)`, in message order, for storing.
    pub fn entries(&self) -> Vec<(u64, DataClass)> {
        self.0.iter().map(|(m, c)| (m.0, *c)).collect()
    }

    /// Mark `msg` at least `class`. Lowering is a no-op.
    pub fn raise(&mut self, msg: MsgId, class: DataClass) {
        let slot = self.0.entry(msg).or_insert(class);
        *slot = (*slot).max(class);
    }

    pub fn of(&self, msg: MsgId) -> Option<DataClass> {
        self.0.get(&msg).copied()
    }

    /// Raise `card` and each of its evidence items to their taint. A
    /// tainted item raises the card too: the card shows it.
    pub fn apply(&self, card: &mut Card) {
        let lift = |c: DataClass, m: MsgId| self.of(m).map_or(c, |t| c.max(t));
        card.class = lift(card.class, card.id);
        for item in &mut card.evidence {
            item.class = lift(item.class.max(card.class), item.msg);
        }
        if let Some(top) = card.evidence.iter().map(|i| i.class).max() {
            card.class = card.class.max(top);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::card::fixtures::{card, item};
    use crate::context::{Line, Phase};
    use rusty_bbp::{Direction, PrincipalId};
    use DataClass::*;

    #[test]
    fn taint_only_goes_up() {
        let mut t = Taints::default();
        t.raise(MsgId(4), Sensitive);
        t.raise(MsgId(4), Public);
        assert_eq!(t.of(MsgId(4)), Some(Sensitive));
        let back = Taints::from_entries(t.entries());
        assert_eq!(back, t);
    }

    #[test]
    fn a_tainted_item_raises_its_card() {
        let mut c = card(vec![
            item(10, "a", Direction::Support, Internal),
            item(11, "b", Direction::Counter, Internal),
        ]);
        let t = Taints::from_entries([(11, Sensitive)]);
        t.apply(&mut c);
        assert_eq!(c.class, Sensitive);
        assert_eq!(c.evidence[1].class, Sensitive);
    }

    #[test]
    fn a_context_is_as_high_as_what_it_admits() {
        let c = card(vec![item(10, "a", Direction::Support, Internal)]);
        let lines = [
            Line {
                text: "x".into(),
                class: Sensitive,
            },
            Line {
                text: "y".into(),
                class: Public,
            },
        ];
        let lens = PrincipalId("l".into());
        let parts = Parts {
            prefix: "",
            card: &c,
            lens: &lens,
            phase: Phase::Informed,
            lines: &lines,
            record: None,
            suffix: None,
        };
        assert_eq!(of_context(&parts, Sensitive), Sensitive);
        assert_eq!(of_context(&parts, Internal), Internal);
    }
}
