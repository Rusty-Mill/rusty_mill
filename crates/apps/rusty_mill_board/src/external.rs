//! Observation-only pointers to execution that does not run under Rusty Orch (ADR-0006).
//!
//! A card records which Codex Cloud thread, GitHub PR or Actions run worked on it and the
//! state an observer last saw. Nothing on the board reads these to decide anything: taking,
//! moving, claiming and the Orch execution link ignore them. Pure, like [`crate::domain`]:
//! callers pass `now_ms`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::domain::{BoardError, Card};

/// At most this many refs per card; see [`Card::observe`] for which one a new ref replaces.
pub const MAX_EXTERNAL_REFS: usize = 8;
/// An observation may be at most this far ahead of the board's clock, so a fast observer
/// clock cannot pin a state that later observations can never replace.
pub const MAX_CLOCK_AHEAD_MS: i64 = 5 * 60 * 1000;
const MAX_REF_CHARS: usize = 128;
const MAX_URL_BYTES: usize = 512;

/// Where the execution ran. Orch and BBP are deliberately absent: their ids belong in the
/// card's [`ExecutionLink`](crate::domain::ExecutionLink).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Provider {
    /// A Codex Cloud thread; its latest turn is the thread's state.
    CodexCloud,
    /// A GitHub pull request, `owner/repo#123`.
    GithubPr,
    /// A GitHub Actions run.
    GithubActions,
}

/// The state an observer saw.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RefState {
    /// Waiting to start.
    Queued,
    /// In progress.
    Running,
    /// Finished and succeeded (a PR: merged).
    Succeeded,
    /// Finished and failed.
    Failed,
    /// Stopped before finishing (a PR: closed unmerged).
    Cancelled,
    /// The observer could not tell; treated as possibly still running.
    Unknown,
}

impl RefState {
    /// Whether the execution is known to have finished.
    #[must_use]
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Cancelled)
    }
}

/// One execution outside Orch and its last observed state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalRef {
    /// Where it ran.
    pub provider: Provider,
    /// The provider's own id: a thread id, `owner/repo#123`, a run id.
    #[serde(rename = "ref")]
    pub reference: String,
    /// A link to it, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// The state last observed.
    pub state: RefState,
    /// When the observer saw that state (Unix ms, the observer's clock).
    pub observed_at_ms: i64,
    /// Who recorded it: the acting identity, never the caller's claim.
    pub by: String,
}

/// What an observer reports: `card_execution_observe`'s input after parsing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observation {
    /// Where it ran.
    pub provider: Provider,
    /// The provider's id.
    pub reference: String,
    /// The state seen.
    pub state: RefState,
    /// A link, if known.
    pub url: Option<String>,
    /// When it was seen (Unix ms).
    pub observed_at_ms: i64,
}

impl Observation {
    /// Refuse input outside ADR-0006's bounds, `now_ms` being the board's clock.
    ///
    /// # Errors
    /// [`BoardError::Invalid`] naming the field.
    pub fn validate(&self, now_ms: i64) -> Result<(), BoardError> {
        let chars = self.reference.chars().count();
        let allowed = |c: char| c.is_ascii_alphanumeric() || "._:/#-".contains(c);
        if chars == 0 || chars > MAX_REF_CHARS || !self.reference.chars().all(allowed) {
            return Err(BoardError::Invalid(
                "ref must be 1-128 characters of A-Z a-z 0-9 . _ : / # -",
            ));
        }
        if let Some(url) = &self.url {
            let clean = !url.chars().any(|c| c.is_whitespace() || c.is_control());
            if url.len() > MAX_URL_BYTES || !url.starts_with("https://") || !clean {
                return Err(BoardError::Invalid(
                    "url must be an https:// link of at most 512 bytes",
                ));
            }
        }
        if self.observed_at_ms < 0 {
            return Err(BoardError::Invalid("observed_at_ms is negative"));
        }
        if self.observed_at_ms > now_ms.saturating_add(MAX_CLOCK_AHEAD_MS) {
            return Err(BoardError::Invalid(
                "observed_at_ms is more than 5 minutes ahead of the board's clock",
            ));
        }
        Ok(())
    }

    fn same_target(&self, r: &ExternalRef) -> bool {
        r.provider == self.provider && r.reference == self.reference
    }

    fn into_ref(self, by: &str, url: Option<String>) -> ExternalRef {
        ExternalRef {
            provider: self.provider,
            reference: self.reference,
            url: self.url.or(url),
            state: self.state,
            observed_at_ms: self.observed_at_ms,
            by: by.to_owned(),
        }
    }
}

impl Card {
    /// Record `obs`, by `by`, upserting by `(provider, ref)`.
    ///
    /// - An older observation than the stored one, or the same one again, returns the card
    ///   unchanged, so observations may arrive late or twice.
    /// - The same time with a different state or url is refused: re-observe with a fresh time.
    /// - A newer observation replaces the stored state; one without a url keeps the stored url.
    /// - A new ref past [`MAX_EXTERNAL_REFS`] replaces the finished ref observed longest ago,
    ///   and is refused when none has finished: a live or `unknown` ref is never dropped.
    ///
    /// # Errors
    /// Input refused by [`Observation::validate`], a same-time conflict, or a full list.
    pub fn observe(&self, obs: Observation, by: &str, now_ms: i64) -> Result<Self, BoardError> {
        obs.validate(now_ms)?;
        let mut refs = self.external_refs.clone();
        match refs.iter().position(|r| obs.same_target(r)) {
            Some(i) => {
                let stored = &refs[i];
                if obs.observed_at_ms < stored.observed_at_ms {
                    return Ok(self.clone());
                }
                if obs.observed_at_ms == stored.observed_at_ms {
                    let same_url = obs.url.is_none() || obs.url == stored.url;
                    if obs.state == stored.state && same_url {
                        return Ok(self.clone());
                    }
                    return Err(BoardError::Refused(
                        "this ref already has a different observation at that time; observe \
                         again with a fresh observed_at_ms"
                            .into(),
                    ));
                }
                let url = stored.url.clone();
                refs[i] = obs.into_ref(by, url);
            }
            None => {
                if refs.len() >= MAX_EXTERNAL_REFS {
                    let oldest_finished = refs
                        .iter()
                        .enumerate()
                        .filter(|(_, r)| r.state.is_terminal())
                        .min_by_key(|(_, r)| r.observed_at_ms)
                        .map(|(i, _)| i)
                        .ok_or_else(|| {
                            BoardError::Refused(
                                "card already has 8 external refs and none has finished".into(),
                            )
                        })?;
                    refs.remove(oldest_finished);
                }
                refs.push(obs.into_ref(by, None));
            }
        }
        Ok(Self {
            external_refs: refs,
            ..self.next(now_ms)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_000_000_000;

    fn card() -> Card {
        Card::new("t", "", vec![], 0).expect("card")
    }

    fn obs(reference: &str, state: RefState, at: i64) -> Observation {
        Observation {
            provider: Provider::CodexCloud,
            reference: reference.into(),
            state,
            url: None,
            observed_at_ms: at,
        }
    }

    fn observed(c: &Card, o: Observation) -> Card {
        c.observe(o, "codex", NOW).expect("observe")
    }

    #[test]
    fn a_new_ref_is_recorded_with_the_acting_identity() {
        let c = observed(&card(), obs("thread-1", RefState::Running, NOW));
        assert_eq!(c.rev, 1);
        let r = &c.external_refs[0];
        assert_eq!(
            (r.reference.as_str(), r.state, r.by.as_str()),
            ("thread-1", RefState::Running, "codex")
        );
    }

    #[test]
    fn replaying_or_observing_older_returns_the_card_unchanged() {
        let c = observed(&card(), obs("t", RefState::Succeeded, NOW));
        assert_eq!(observed(&c, obs("t", RefState::Succeeded, NOW)), c);
        assert_eq!(observed(&c, obs("t", RefState::Running, NOW - 1)), c);
    }

    #[test]
    fn a_newer_observation_replaces_the_state_and_keeps_the_url() {
        let first = Observation {
            url: Some("https://example.test/t".into()),
            ..obs("t", RefState::Running, NOW - 10)
        };
        let c = observed(&observed(&card(), first), obs("t", RefState::Failed, NOW));
        assert_eq!(c.external_refs.len(), 1);
        assert_eq!(c.external_refs[0].state, RefState::Failed);
        assert_eq!(
            c.external_refs[0].url.as_deref(),
            Some("https://example.test/t")
        );
    }

    #[test]
    fn the_same_time_with_a_different_state_is_refused() {
        let c = observed(&card(), obs("t", RefState::Running, NOW));
        assert!(matches!(
            c.observe(obs("t", RefState::Failed, NOW), "codex", NOW),
            Err(BoardError::Refused(_))
        ));
    }

    #[test]
    fn providers_keep_refs_apart() {
        let c = observed(&card(), obs("1", RefState::Running, NOW));
        let pr = Observation {
            provider: Provider::GithubPr,
            ..obs("1", RefState::Queued, NOW)
        };
        assert_eq!(observed(&c, pr).external_refs.len(), 2);
    }

    #[test]
    fn input_outside_the_bounds_is_invalid() {
        let bad = [
            obs("", RefState::Running, NOW),
            obs("has space", RefState::Running, NOW),
            obs(&"x".repeat(129), RefState::Running, NOW),
            obs("t", RefState::Running, NOW + MAX_CLOCK_AHEAD_MS + 1),
            obs("t", RefState::Running, -1),
            Observation {
                url: Some("http://example.test".into()),
                ..obs("t", RefState::Running, NOW)
            },
            Observation {
                url: Some(format!("https://{}", "a".repeat(512))),
                ..obs("t", RefState::Running, NOW)
            },
            Observation {
                url: Some("https://example.test/a b".into()),
                ..obs("t", RefState::Running, NOW)
            },
        ];
        for o in bad {
            assert!(
                matches!(
                    card().observe(o.clone(), "codex", NOW),
                    Err(BoardError::Invalid(_))
                ),
                "{o:?}"
            );
        }
        let edge = obs(
            &"a".repeat(128),
            RefState::Running,
            NOW + MAX_CLOCK_AHEAD_MS,
        );
        assert!(card().observe(edge, "codex", NOW).is_ok());
        assert!(
            card()
                .observe(obs("owner/repo#12", RefState::Queued, NOW), "c", NOW)
                .is_ok()
        );
    }

    fn full(states: [RefState; MAX_EXTERNAL_REFS]) -> Card {
        states.iter().zip(0..).fold(card(), |c, (s, i)| {
            observed(&c, obs(&format!("r{i}"), *s, NOW - 100 + i))
        })
    }

    #[test]
    fn the_ninth_ref_replaces_the_oldest_finished_ref() {
        use RefState::*;
        let c = full([
            Running, Succeeded, Unknown, Failed, Queued, Running, Running, Cancelled,
        ]);
        let c = observed(&c, obs("new", Running, NOW));
        let refs: Vec<_> = c
            .external_refs
            .iter()
            .map(|r| r.reference.as_str())
            .collect();
        assert_eq!(refs, ["r0", "r2", "r3", "r4", "r5", "r6", "r7", "new"]);
    }

    #[test]
    fn the_ninth_ref_is_refused_when_none_has_finished() {
        use RefState::*;
        let c = full([
            Running, Unknown, Queued, Running, Unknown, Queued, Running, Unknown,
        ]);
        assert!(matches!(
            c.observe(obs("new", Succeeded, NOW), "codex", NOW),
            Err(BoardError::Refused(_))
        ));
        // An existing ref still updates on a full card.
        assert_eq!(
            observed(&c, obs("r0", Succeeded, NOW)).external_refs.len(),
            8
        );
    }
}
