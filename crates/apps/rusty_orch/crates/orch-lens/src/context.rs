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
use crate::compose::Plan;

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

/// A lens's prefix around its lens prompt: intent and board rules
/// before it, the reply protocol after. `wrap` gives [`prefix`]'s bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    head: String,
    tail: String,
}

impl Frame {
    /// The frame for a lens under `intent` and `rules`.
    pub fn new(intent: &str, rules: &str) -> Self {
        Self {
            head: format!("# Intent\n{intent}\n\n# Board rules\n{rules}\n\n# Your lens\n"),
            tail: format!("\n\n# Reply\n{}", crate::post::protocol()),
        }
    }

    /// The prefix with `lens_prompt` in its place.
    pub fn wrap(&self, lens_prompt: &str) -> String {
        format!("{}{lens_prompt}{}", self.head, self.tail)
    }

    fn bytes(&self) -> usize {
        self.head.len() + self.tail.len()
    }
}

/// A job's prefix.
#[derive(Debug, Clone, Copy)]
pub enum Prefix<'a> {
    /// Fixed bytes.
    Flat(&'a str),
    /// A composed lens prompt in its frame (#723): when prefix and card do
    /// not fit a provider's window, the plan's sections are left out in
    /// order, down to the bare lens prompt, and only the chosen prefix is
    /// rendered.
    Composed { frame: &'a Frame, plan: &'a Plan },
}

/// Everything one job's context is made from.
#[derive(Debug, Clone, Copy)]
pub struct Parts<'a> {
    pub prefix: Prefix<'a>,
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
    Frame::new(intent, rules).wrap(lens_prompt)
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
/// cut: a composed prefix leaves out the fewest sections, in their order,
/// that let it fit with the card and suffix before the card is refused.
/// Settled lines and the track record take what is left.
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
    let (prefix, dropped) = choose(p.prefix, room, card.len() + suffix.len())?;
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
        prefix,
        body,
        dropped,
    })
}

/// The prefix that fits `room` beside `rest` bytes, and the sections left
/// out for it. Only that prefix is rendered.
fn choose(prefix: Prefix<'_>, room: u64, rest: usize) -> Result<(String, Vec<String>), Refusal> {
    let need = |len: usize| (len + rest) as u64;
    match prefix {
        Prefix::Flat(p) if need(p.len()) <= room => Ok((p.to_owned(), Vec::new())),
        Prefix::Flat(p) => Err(Refusal::TooLarge {
            needed: need(p.len()),
            room,
            dropped: Vec::new(),
        }),
        Prefix::Composed { frame, plan } => {
            let at = |k: usize| need(frame.bytes() + plan.bytes(k));
            match (0..=plan.steps()).find(|&k| at(k) <= room) {
                Some(k) => Ok((frame.wrap(&plan.render(k)), plan.left_out(k))),
                None => Err(Refusal::TooLarge {
                    needed: at(plan.steps()),
                    room,
                    dropped: plan.left_out(plan.steps()),
                }),
            }
        }
    }
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
            prefix: Prefix::Flat(&pre),
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
            prefix: Prefix::Flat("p"),
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
            prefix: Prefix::Flat("p"),
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
            prefix: Prefix::Flat("p"),
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
            prefix: Prefix::Flat("p"),
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
            prefix: Prefix::Flat("p"),
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
        use crate::compose::{Limits, Selection, Text, RENDERS};
        use std::collections::{BTreeMap, BTreeSet};

        fn plan(files: &[(&str, String)], limits: Limits) -> Plan {
            let texts: BTreeMap<String, Text> = files
                .iter()
                .map(|(p, t)| ((*p).to_owned(), Text::Loaded(t.clone())))
                .collect();
            let pick = |dir: &str| -> BTreeSet<String> {
                texts
                    .keys()
                    .filter(|p| p.starts_with(dir))
                    .cloned()
                    .collect()
            };
            let sel = Selection {
                knowledge: pick("knowledge/"),
                standards: pick("standards/"),
            };
            Plan::new("find faults", None, &sel, &texts, limits).expect("plans")
        }

        fn two() -> Plan {
            plan(
                &[
                    ("knowledge/rust.md", "K".repeat(400)),
                    ("standards/rust.md", "S".repeat(200)),
                ],
                Limits::default(),
            )
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

        fn frame() -> Frame {
            Frame::new("the brief", "rules")
        }

        fn fitted(plan: &Plan, w: Window) -> Result<Fitted, Refusal> {
            let c = card(vec![]);
            let me = lens();
            let frame = frame();
            let parts = Parts {
                prefix: Prefix::Composed {
                    frame: &frame,
                    plan,
                },
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

        /// Bytes of prefix and card with `k` sections left out.
        fn need(plan: &Plan, k: usize) -> u64 {
            (frame().wrap(&plan.render(k)).len() + card_text().len()) as u64
        }

        #[test]
        fn a_frame_wraps_to_the_flat_prefix() {
            assert_eq!(
                frame().wrap("find faults"),
                prefix("the brief", "rules", "find faults")
            );
        }

        #[test]
        fn the_full_composition_is_used_when_it_fits() {
            let p = two();
            let f = fitted(&p, window(need(&p, 0), true)).expect("fits");
            assert_eq!(f.prefix, frame().wrap(&p.render(0)));
            assert!(f.prefix.contains(&"K".repeat(400)));
            assert!(f.dropped.is_empty());
            assert_eq!(f.body, card_text());
        }

        #[test]
        fn sections_drop_in_order_until_the_card_fits() {
            let p = two();
            let f = fitted(&p, window(need(&p, 0) - 1, false)).expect("fits trimmed");
            assert_eq!(f.prefix, frame().wrap(&p.render(1)));
            assert!(f.prefix.contains(&"S".repeat(200)) && !f.prefix.contains('K'));
            assert_eq!(f.dropped, vec!["knowledge/rust.md".to_owned()]);
            let f = fitted(&p, window(need(&p, 2), true)).expect("base fits");
            assert_eq!(f.prefix, prefix("the brief", "rules", "find faults"));
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
            let p = two();
            let w = window(need(&p, 2) - 1, false);
            let room = u64::from(w.context_tokens - w.max_output_tokens) * BYTES_PER_TOKEN;
            assert_eq!(
                fitted(&p, w),
                Err(Refusal::TooLarge {
                    needed: need(&p, 2),
                    room,
                    dropped: vec![
                        "knowledge/rust.md".to_owned(),
                        "standards/rust.md".to_owned()
                    ],
                })
            );
        }

        #[test]
        fn many_small_sections_render_only_the_chosen_prefix() {
            let files: Vec<(String, String)> = (0..2000)
                .map(|i| (format!("knowledge/{i:04}.md"), "x".to_owned()))
                .collect();
            let files: Vec<(&str, String)> =
                files.iter().map(|(p, t)| (p.as_str(), t.clone())).collect();
            let limits = Limits {
                file_bytes: 64,
                total_bytes: 1 << 20,
            };
            let p = plan(&files, limits);
            assert_eq!(p.steps(), 2000);
            let base = frame().bytes() + card_text().len();
            let room = (base + p.bytes(1000)) as u64;
            RENDERS.with(|n| n.set(0));
            let f = fitted(&p, window(room, false)).expect("fits trimmed");
            assert_eq!(RENDERS.with(|n| n.get()), 1, "one prefix rendered");
            let k = f.dropped.len();
            assert!((1000..1004).contains(&k), "{k}");
            assert_eq!(f.prefix.len(), frame().bytes() + p.bytes(k));
            assert_eq!(
                f.dropped.first().map(String::as_str),
                Some("knowledge/1999.md")
            );
            RENDERS.with(|n| n.set(0));
            let refused = fitted(&p, window(base as u64 - 1, false));
            assert!(matches!(refused, Err(Refusal::TooLarge { .. })));
            assert_eq!(RENDERS.with(|n| n.get()), 0, "a refusal renders nothing");
        }
    }
}
