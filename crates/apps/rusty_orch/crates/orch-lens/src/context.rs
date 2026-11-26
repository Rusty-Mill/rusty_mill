//! Context assembly: a fresh question per job, built from the board alone.
//!
//! 1. The prefix: intent, board rules, lens prompt. Byte-identical across
//!    every job of one lens, so providers can cache it.
//! 2. The claim card, blind or informed.
//! 3. Settled claims and deps as one line each, newest first, capped.
//! 4. The lens's track record, one line.
//! 5. An optional job suffix (look harder, rebel).
//!
//! The whole must fit the provider's window less its output allowance,
//! estimated at 4 bytes a token. Settled lines go first, oldest first, then
//! the track record; the card is never cut. A card that does not fit is a
//! refusal for that provider.

use rusty_bbp::PrincipalId;

use crate::card::{render, Card, Show};
use crate::class::{admit, DataClass};

/// Default cap on settled lines.
pub const MAX_LINES: usize = 40;
/// Bytes per token, for estimates.
pub const BYTES_PER_TOKEN: u64 = 4;

/// Blind first pass or informed follow-up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Blind,
    Informed,
}

/// A settled claim or dep, already compacted by BBP's `claim_line`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub text: String,
    pub class: DataClass,
}

/// A smaller prefix a job may fall back to (#723), and what it leaves out
/// compared with the job's full prefix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rung {
    pub prefix: String,
    pub dropped: Vec<String>,
}

/// Everything one job's context is made from.
#[derive(Debug, Clone, Copy)]
pub struct Parts<'a> {
    pub prefix: &'a str,
    /// Smaller prefixes, largest first, the last being the base prefix
    /// with no composed sections. Tried in order when `prefix` and the
    /// card do not fit a provider's window. Empty when nothing is composed.
    pub smaller: &'a [Rung],
    pub card: &'a Card,
    pub lens: &'a PrincipalId,
    pub phase: Phase,
    /// Newest first.
    pub lines: &'a [Line],
    pub record: Option<&'a str>,
    pub suffix: Option<&'a str>,
}

/// One provider's limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Window {
    pub context_tokens: u32,
    pub max_output_tokens: u32,
    pub clearance: DataClass,
}

/// Why a context could not be built for a provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// The claim itself is above the provider's clearance.
    AboveClearance {
        card: DataClass,
        clearance: DataClass,
    },
    /// Prefix and card alone exceed the window, even with every composed
    /// section left out (`dropped`).
    TooLarge {
        needed: u64,
        room: u64,
        dropped: Vec<String>,
    },
}

/// A context that fits, and the composed sections left out to make it fit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fitted {
    pub prefix: String,
    pub body: String,
    pub dropped: Vec<String>,
}

/// The prefix: intent, rules, lens prompt and the reply protocol, in that
/// order.
pub fn prefix(intent: &str, rules: &str, lens_prompt: &str) -> String {
    format!(
        "{}\n# Reply\n{}",
        raw_prefix(intent, rules, lens_prompt),
        crate::post::protocol()
    )
}

/// The prefix of a lens whose replies are not board posts (the #708 alien
/// lens): intent, rules and lens prompt, with no reply protocol, since
/// each question body says what to reply.
pub fn raw_prefix(intent: &str, rules: &str, lens_prompt: &str) -> String {
    format!("# Intent\n{intent}\n\n# Board rules\n{rules}\n\n# Your lens\n{lens_prompt}\n")
}

/// Build `(prefix, body)` for one provider; see [`fit`].
pub fn assemble(p: &Parts<'_>, w: Window) -> Result<(String, String), Refusal> {
    fit(p, w).map(|f| (f.prefix, f.body))
}

/// Build one provider's context. The card and the base prefix are never
/// cut: the first of `prefix` then `smaller` that fits with the card and
/// suffix is used, so composed sections drop in their order before the
/// card is refused. Settled lines and the track record take what is left.
pub fn fit(p: &Parts<'_>, w: Window) -> Result<Fitted, Refusal> {
    if !admit(p.card.class, w.clearance) {
        return Err(Refusal::AboveClearance {
            card: p.card.class,
            clearance: w.clearance,
        });
    }
    let room = u64::from(w.context_tokens.saturating_sub(w.max_output_tokens)) * BYTES_PER_TOKEN;
    let show = match p.phase {
        Phase::Blind => Show::OwnOnly(p.lens),
        Phase::Informed => Show::All,
    };
    let card = render(p.card, show, w.clearance);
    let suffix = p.suffix.map(|s| format!("\n{s}\n")).unwrap_or_default();
    let full = (p.prefix, [].as_slice());
    let rungs = std::iter::once(full).chain(
        p.smaller
            .iter()
            .map(|r| (r.prefix.as_str(), r.dropped.as_slice())),
    );
    let mut last = full;
    let mut chosen = None;
    for rung in rungs {
        last = rung;
        if (rung.0.len() + card.len() + suffix.len()) as u64 <= room {
            chosen = Some(rung);
            break;
        }
    }
    let Some((prefix, dropped)) = chosen else {
        return Err(Refusal::TooLarge {
            needed: (last.0.len() + card.len() + suffix.len()) as u64,
            room,
            dropped: last.1.to_vec(),
        });
    };
    let mut left = room - (prefix.len() + card.len() + suffix.len()) as u64;
    let record = p
        .record
        .map(|r| format!("\nYour track record: {r}\n"))
        .filter(|r| fits(&mut left, r.len()));
    let lines = settled(p.lines, w.clearance, &mut left);
    let mut body = card;
    if !lines.is_empty() {
        body.push_str("\nSettled and related claims:\n");
        body.push_str(&lines);
    }
    body.push_str(&record.unwrap_or_default());
    body.push_str(&suffix);
    Ok(Fitted {
        prefix: prefix.to_owned(),
        body,
        dropped: dropped.to_vec(),
    })
}

/// The admitted lines, newest first, as many as fit after the record.
fn settled(lines: &[Line], clearance: DataClass, left: &mut u64) -> String {
    const HEADER: usize = "\nSettled and related claims:\n".len();
    if !fits(left, HEADER) {
        return String::new();
    }
    let mut out = String::new();
    for l in lines
        .iter()
        .filter(|l| admit(l.class, clearance))
        .take(MAX_LINES)
    {
        let line = format!("{}\n", l.text);
        if !fits(left, line.len()) {
            break;
        }
        out.push_str(&line);
    }
    if out.is_empty() {
        *left += HEADER as u64;
    }
    out
}

fn fits(left: &mut u64, len: usize) -> bool {
    let len = len as u64;
    if len > *left {
        return false;
    }
    *left -= len;
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::card::fixtures::{card, item};
    use rusty_bbp::Direction;

    fn lens() -> PrincipalId {
        PrincipalId("adversary".to_owned())
    }

    fn lines(n: usize) -> Vec<Line> {
        (0..n)
            .map(|i| Line {
                text: format!("msg:{} ACCEPTED +8 (90%) 3 evd · line {i}", 100 + i),
                class: DataClass::Internal,
            })
            .collect()
    }

    const WIDE: Window = Window {
        context_tokens: 32_768,
        max_output_tokens: 2_048,
        clearance: DataClass::Internal,
    };

    #[test]
    fn the_prefix_is_identical_across_claims() {
        let pre = prefix("dedup 20 TB", "rules", "find faults");
        let (a, b) = (card(vec![]), {
            let mut c = card(vec![]);
            c.id = rusty_bbp::MsgId(8);
            c
        });
        let ls = lines(3);
        let me = lens();
        let parts = |c| Parts {
            prefix: &pre,
            smaller: &[],
            card: c,
            lens: &me,
            phase: Phase::Blind,
            lines: &ls,
            record: None,
            suffix: None,
        };
        let (pa, _) = assemble(&parts(&a), WIDE).expect("fits");
        let (pb, _) = assemble(&parts(&b), WIDE).expect("fits");
        assert_eq!(pa, pb);
    }

    #[test]
    fn blind_hides_foreign_evidence_and_informed_shows_it() {
        let c = card(vec![item(
            9,
            "operator",
            Direction::Support,
            DataClass::Internal,
        )]);
        let me = lens();
        let mut parts = Parts {
            prefix: "p",
            smaller: &[],
            card: &c,
            lens: &me,
            phase: Phase::Blind,
            lines: &[],
            record: None,
            suffix: None,
        };
        let (_, blind) = assemble(&parts, WIDE).expect("fits");
        assert!(!blind.contains("note 9"));
        parts.phase = Phase::Informed;
        let (_, informed) = assemble(&parts, WIDE).expect("fits");
        assert!(informed.contains("note 9"));
    }

    #[test]
    fn lines_are_capped_and_newest_first() {
        let c = card(vec![]);
        let me = lens();
        let ls = lines(MAX_LINES + 5);
        let parts = Parts {
            prefix: "p",
            smaller: &[],
            card: &c,
            lens: &me,
            phase: Phase::Informed,
            lines: &ls,
            record: Some("4 aligned, 1 misaligned"),
            suffix: Some("Look harder."),
        };
        let (_, body) = assemble(&parts, WIDE).expect("fits");
        assert_eq!(body.matches(" ACCEPTED ").count(), MAX_LINES);
        assert!(body.contains("line 0") && !body.contains(&format!("line {MAX_LINES}")));
        assert!(body.contains("Your track record: 4 aligned"));
        assert!(body.trim_end().ends_with("Look harder."));
    }

    #[test]
    fn a_tight_window_drops_lines_oldest_first_then_the_record() {
        let c = card(vec![]);
        let me = lens();
        let ls = lines(10);
        let parts = Parts {
            prefix: "p",
            smaller: &[],
            card: &c,
            lens: &me,
            phase: Phase::Informed,
            lines: &ls,
            record: Some("4 aligned"),
            suffix: None,
        };
        let card_len = render(&c, Show::All, DataClass::Internal).len() as u64;
        // Room for the card, the record and about two lines.
        let tokens = |bytes: u64| (bytes / BYTES_PER_TOKEN + 1) as u32;
        let w = Window {
            context_tokens: tokens(card_len + 1 + 30 + 30 + 2 * 45),
            max_output_tokens: 0,
            ..WIDE
        };
        let (_, body) = assemble(&parts, w).expect("fits");
        assert!(body.contains("line 0"), "{body}");
        assert!(!body.contains("line 9"), "{body}");
        assert!(body.contains("track record"), "{body}");

        let only_card = Window {
            context_tokens: tokens(card_len + 1),
            max_output_tokens: 0,
            ..WIDE
        };
        let (_, body) = assemble(&parts, only_card).expect("card alone fits");
        assert!(
            !body.contains("line 0") && !body.contains("track record"),
            "{body}"
        );
    }

    #[test]
    fn an_oversized_card_or_one_above_clearance_is_refused() {
        let c = card(vec![]);
        let me = lens();
        let parts = Parts {
            prefix: "p",
            smaller: &[],
            card: &c,
            lens: &me,
            phase: Phase::Informed,
            lines: &[],
            record: None,
            suffix: None,
        };
        let tiny = Window {
            context_tokens: 10,
            max_output_tokens: 0,
            ..WIDE
        };
        assert!(matches!(
            assemble(&parts, tiny),
            Err(Refusal::TooLarge { .. })
        ));
        let mut secret = c.clone();
        secret.class = DataClass::Sensitive;
        let parts = Parts {
            card: &secret,
            ..parts
        };
        assert_eq!(
            assemble(&parts, WIDE),
            Err(Refusal::AboveClearance {
                card: DataClass::Sensitive,
                clearance: DataClass::Internal
            })
        );
    }

    #[test]
    fn lines_above_clearance_never_appear() {
        let c = card(vec![]);
        let me = lens();
        let ls = vec![Line {
            text: "msg:50 ACCEPTED secret path".to_owned(),
            class: DataClass::Sensitive,
        }];
        let parts = Parts {
            prefix: "p",
            smaller: &[],
            card: &c,
            lens: &me,
            phase: Phase::Informed,
            lines: &ls,
            record: None,
            suffix: None,
        };
        let (_, body) = assemble(&parts, WIDE).expect("fits");
        assert!(
            !body.contains("secret path") && !body.contains("Settled"),
            "{body}"
        );
    }

    mod composed {
        use super::*;

        const BASE: &str = "# Your lens\nfind faults\n# Reply\n";

        fn rungs() -> (String, Vec<Rung>) {
            let full = format!("{BASE}{}", "K".repeat(400));
            let rung = |extra: usize, dropped: &[&str]| Rung {
                prefix: format!("{BASE}{}", "S".repeat(extra)),
                dropped: dropped.iter().map(|d| (*d).to_owned()).collect(),
            };
            let smaller = vec![
                rung(200, &["knowledge/rust.md"]),
                rung(0, &["knowledge/rust.md", "standards/rust.md"]),
            ];
            (full, smaller)
        }

        /// A window whose room, after the output allowance, is `bytes`
        /// rounded up (`up`) or down to whole tokens.
        fn window(bytes: u64, up: bool) -> Window {
            let tokens = if up {
                bytes.div_ceil(BYTES_PER_TOKEN)
            } else {
                bytes / BYTES_PER_TOKEN
            };
            Window {
                context_tokens: tokens as u32 + 10,
                max_output_tokens: 10,
                clearance: DataClass::Internal,
            }
        }

        fn fitted(w: Window) -> Result<Fitted, Refusal> {
            let (full, smaller) = rungs();
            let c = card(vec![]);
            let me = lens();
            let parts = Parts {
                prefix: &full,
                smaller: &smaller,
                card: &c,
                lens: &me,
                phase: Phase::Blind,
                lines: &[],
                record: None,
                suffix: None,
            };
            fit(&parts, w)
        }

        fn card_text() -> String {
            render(&card(vec![]), Show::OwnOnly(&lens()), DataClass::Internal)
        }

        fn base_need() -> u64 {
            (BASE.len() + card_text().len()) as u64
        }

        #[test]
        fn the_full_composition_is_used_when_it_fits() {
            let f = fitted(window(base_need() + 400, true)).expect("fits");
            assert!(f.prefix.ends_with(&"K".repeat(400)));
            assert!(f.dropped.is_empty());
            assert_eq!(f.body, card_text());
        }

        #[test]
        fn sections_drop_in_order_until_the_card_fits() {
            let f = fitted(window(base_need() + 399, false)).expect("fits trimmed");
            assert_eq!(f.prefix, format!("{BASE}{}", "S".repeat(200)));
            assert_eq!(f.dropped, vec!["knowledge/rust.md".to_owned()]);
            let f = fitted(window(base_need(), true)).expect("base fits");
            assert_eq!(f.prefix, BASE);
            assert_eq!(
                f.dropped,
                vec![
                    "knowledge/rust.md".to_owned(),
                    "standards/rust.md".to_owned()
                ]
            );
            assert_eq!(f.body, card_text(), "the card is whole");
        }

        #[test]
        fn base_and_card_too_large_is_refused_with_the_drops() {
            let w = window(base_need() - 1, false);
            let room = u64::from(w.context_tokens - w.max_output_tokens) * BYTES_PER_TOKEN;
            assert_eq!(
                fitted(w),
                Err(Refusal::TooLarge {
                    needed: base_need(),
                    room,
                    dropped: vec![
                        "knowledge/rust.md".to_owned(),
                        "standards/rust.md".to_owned()
                    ],
                })
            );
        }
    }
}
