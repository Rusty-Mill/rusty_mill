//! The lens runner's [`Model`] port over [`OpenAiHttp`] (#594).

use orch_dispatch::{AgentError, ClassifiedError};
use orch_lens::{parse, Allowed, Answer, Model, Question};

use crate::client::{OpenAiHttp, Resolve};
use crate::wire::{Completion, Prompt, Usage};

impl<R: Resolve> Model for OpenAiHttp<R> {
    /// One chat completion: the question's prefix as the system message,
    /// its body as the user message. The reply must pass [`parse`].
    fn ask(&mut self, q: &Question) -> Result<Answer, ClassifiedError> {
        let prompt = Prompt {
            system: &q.prefix,
            user: &q.body,
            max_output_tokens: q.max_output_tokens,
        };
        answer(self.complete(&prompt)?, &q.allowed)
    }
}

/// Check a completion against what the job may post.
///
/// A non-conforming reply is `Permanent`, so nothing unvalidated reaches
/// the board. Its usage is lost with it; the runner then settles the
/// call's full reservation, which bounds it.
pub fn answer(c: Completion, allowed: &Allowed) -> Result<Answer, ClassifiedError> {
    let posts = parse(&c.text, allowed)
        .map_err(|e| ClassifiedError::Permanent(AgentError(format!("openai reply: {e}"))))?;
    Ok(Answer {
        posts,
        usage: usage(c.usage),
    })
}

/// Cached tokens are a part of the input; a server that reports more is
/// held to the input count, so no cost is computed on phantom tokens.
fn usage(u: Usage) -> orch_lens::Usage {
    orch_lens::Usage {
        input_tokens: u.input_tokens,
        output_tokens: u.output_tokens,
        cached_input_tokens: u.cached_input_tokens.unwrap_or(0).min(u.input_tokens),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn none() -> Allowed {
        Allowed {
            evidence: None,
            claims: None,
        }
    }

    fn completion(text: &str, cached: Option<u64>) -> Completion {
        Completion {
            text: text.to_owned(),
            usage: Usage {
                input_tokens: 40,
                output_tokens: 5,
                cached_input_tokens: cached,
            },
        }
    }

    #[test]
    fn a_conforming_reply_becomes_an_answer_with_usage() {
        let a = answer(completion(r#"{"posts":[]}"#, Some(30)), &none()).unwrap();
        assert!(a.posts.is_empty());
        assert_eq!(
            a.usage,
            orch_lens::Usage {
                input_tokens: 40,
                output_tokens: 5,
                cached_input_tokens: 30,
            }
        );
    }

    #[test]
    fn a_non_conforming_reply_is_permanent() {
        for text in [
            "not json",
            r#"{"posts":[],"extra":1}"#,
            r#"{"posts":[{"kind":"claim"}]}"#,
        ] {
            let err = answer(completion(text, None), &none()).unwrap_err();
            assert!(
                matches!(err, ClassifiedError::Permanent(_)),
                "{text}: {err:?}"
            );
        }
    }

    #[test]
    fn missing_or_excess_cached_counts_are_bounded() {
        let a = answer(completion(r#"{"posts":[]}"#, None), &none()).unwrap();
        assert_eq!(a.usage.cached_input_tokens, 0);
        let a = answer(completion(r#"{"posts":[]}"#, Some(99)), &none()).unwrap();
        assert_eq!(a.usage.cached_input_tokens, 40);
    }
}
