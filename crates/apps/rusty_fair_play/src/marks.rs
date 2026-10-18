//! The family's own marks on a card: its points (how much work it is, on a
//! Fibonacci scale) and its frequency (how often it comes round). Neither
//! is part of the deck, so both live beside the stores like the set-aside
//! list, not on the stored `Card`.
//!
//! A card with no assigned points starts from a suggestion computed from
//! its own text — the words in Conception, Planning and Execution, plus a
//! weight per minimum standard of care — so a short custom or split-off
//! card suggests the least and one that spells out a lot suggests more.
//! An assigned value always wins over the suggestion.

use rusty_fair_play_domain::Card;
use serde::{Deserialize, Serialize};

/// The values a card may be given.
pub const SCALE: [u8; 5] = [1, 2, 3, 5, 8];

/// Each standard of care counts as this many words of description.
const STANDARD_WEIGHT: usize = 8;

/// Score ceilings for 1, 2, 3 and 5 points; anything above is 8. Chosen
/// against the shipped deck (scores of roughly 33 to 62) so it spreads
/// across the scale instead of landing on one value.
const CEILINGS: [usize; 4] = [36, 42, 48, 56];

/// Whether `points` is on the [`SCALE`].
pub fn is_valid(points: u8) -> bool {
    SCALE.contains(&points)
}

/// The points a card suggests from its text and standards of care.
pub fn suggest(card: &Card) -> u8 {
    let words: usize = [&card.conception, &card.planning, &card.execution]
        .iter()
        .map(|text| text.split_whitespace().count())
        .sum();
    let score = words + card.minimum_standard_of_care.len() * STANDARD_WEIGHT;
    CEILINGS
        .iter()
        .position(|ceiling| score <= *ceiling)
        .map_or(SCALE[SCALE.len() - 1], |i| SCALE[i])
}

/// How often a card's work comes round.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Frequency {
    Daily,
    Weekly,
    Monthly,
    Seasonal,
    Yearly,
    AsNeeded,
}

impl Frequency {
    /// Every frequency, most frequent first.
    pub const ALL: [Frequency; 6] = [
        Frequency::Daily,
        Frequency::Weekly,
        Frequency::Monthly,
        Frequency::Seasonal,
        Frequency::Yearly,
        Frequency::AsNeeded,
    ];

    /// The spelling on the wire.
    pub fn as_str(self) -> &'static str {
        match self {
            Frequency::Daily => "Daily",
            Frequency::Weekly => "Weekly",
            Frequency::Monthly => "Monthly",
            Frequency::Seasonal => "Seasonal",
            Frequency::Yearly => "Yearly",
            Frequency::AsNeeded => "As needed",
        }
    }

    /// The inverse of [`Frequency::as_str`].
    pub fn parse(text: &str) -> Option<Frequency> {
        Self::ALL.into_iter().find(|f| f.as_str() == text)
    }
}

/// Phrases that name how often a card's work comes round, by frequency.
/// Matched case-insensitively as substrings, so `season` also finds
/// "seasonal" and `annual` also finds "annually".
const CUES: [(Frequency, &[&str]); 5] = [
    (
        Frequency::Daily,
        &[
            "daily",
            "each day",
            "every day",
            "per day",
            "before bed",
            "bedtime",
            "each night",
            "every night",
            "every morning",
        ],
    ),
    (
        Frequency::Weekly,
        &[
            "weekly",
            "each week",
            "every week",
            "per week",
            "pickup day",
        ],
    ),
    (
        Frequency::Monthly,
        &["monthly", "each month", "every month", "per month"],
    ),
    (Frequency::Seasonal, &["season", "quarterly"]),
    (
        Frequency::Yearly,
        &["annual", "yearly", "each year", "every year", "per year"],
    ),
];

/// The frequency a card recommends: the one its text names most often
/// (a tie goes to the more frequent), else [`Frequency::AsNeeded`] when
/// the text names none. Like [`suggest`], a starting point only.
pub fn suggest_frequency(card: &Card) -> Frequency {
    let text = [
        &card.name,
        &card.conception,
        &card.planning,
        &card.execution,
    ]
    .into_iter()
    .chain(card.minimum_standard_of_care.iter())
    .map(|t| t.to_lowercase())
    .collect::<Vec<_>>()
    .join(
        "
",
    );
    let mut best = (0, Frequency::AsNeeded);
    for (frequency, cues) in CUES {
        let hits: usize = cues.iter().map(|cue| text.matches(cue).count()).sum();
        if hits > best.0 {
            best = (hits, frequency);
        }
    }
    best.1
}

/// What the family has marked on one card. Both absent means nothing is
/// stored for the card.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Marks {
    pub points: Option<u8>,
    pub frequency: Option<Frequency>,
}

impl Marks {
    pub fn is_empty(&self) -> bool {
        self.points.is_none() && self.frequency.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_fair_play_domain::{Card, Origin, Suit};
    use uuid::Uuid;

    fn card(words: usize, standards: usize) -> Card {
        Card {
            id: Uuid::nil(),
            number: None,
            name: "Test".into(),
            suit: Suit::Home,
            parent_card_id: None,
            position: 0,
            owner_id: None,
            conception: vec!["word"; words].join(" "),
            planning: String::new(),
            execution: String::new(),
            minimum_standard_of_care: vec!["standard".to_string(); standards],
            notes: String::new(),
            origin: Origin::Family,
            baseline_id: None,
        }
    }

    #[test]
    fn an_empty_card_suggests_the_least() {
        assert_eq!(suggest(&card(0, 0)), 1);
    }

    #[test]
    fn the_suggestion_steps_up_at_each_ceiling() {
        for (words, want) in [
            (36, 1),
            (37, 2),
            (42, 2),
            (43, 3),
            (48, 3),
            (49, 5),
            (56, 5),
            (57, 8),
        ] {
            assert_eq!(suggest(&card(words, 0)), want, "{words} words");
        }
    }

    #[test]
    fn standards_of_care_weigh_in() {
        assert_eq!(suggest(&card(30, 0)), 1);
        assert_eq!(suggest(&card(30, 1)), 2); // 30 + 8
        assert_eq!(suggest(&card(30, 3)), 5); // 30 + 24
    }

    #[test]
    fn only_scale_values_are_valid() {
        assert!(SCALE.iter().all(|p| is_valid(*p)));
        for bad in [0, 4, 6, 7, 9, 13, 255] {
            assert!(!is_valid(bad), "{bad}");
        }
    }

    fn worded(text: &str) -> Card {
        let mut c = card(0, 0);
        c.execution = text.to_string();
        c
    }

    #[test]
    fn a_card_recommends_the_frequency_its_text_names() {
        for (text, want) in [
            ("Dishwasher run and unloaded daily", Frequency::Daily),
            ("Sink empty before bed", Frequency::Daily),
            ("Floors vacuumed WEEKLY", Frequency::Weekly),
            ("Bins at the curb every pickup day", Frequency::Weekly),
            ("Filters swapped each month", Frequency::Monthly),
            ("Seasonal clothes swapped", Frequency::Seasonal),
            ("Taxes filed annually", Frequency::Yearly),
        ] {
            assert_eq!(suggest_frequency(&worded(text)), want, "{text}");
        }
    }

    #[test]
    fn the_most_named_frequency_wins_and_a_tie_goes_to_the_more_frequent() {
        assert_eq!(
            suggest_frequency(&worded("weekly weekly daily")),
            Frequency::Weekly
        );
        assert_eq!(suggest_frequency(&worded("weekly daily")), Frequency::Daily);
    }

    #[test]
    fn a_card_that_names_no_period_recommends_as_needed() {
        assert_eq!(suggest_frequency(&worded("")), Frequency::AsNeeded);
        assert_eq!(
            suggest_frequency(&worded("Staples never fully run out")),
            Frequency::AsNeeded
        );
    }

    #[test]
    fn frequencies_round_trip_through_their_spelling() {
        for f in Frequency::ALL {
            assert_eq!(Frequency::parse(f.as_str()), Some(f));
        }
        assert_eq!(Frequency::parse("daily"), None);
        assert_eq!(Frequency::parse(""), None);
    }
}
