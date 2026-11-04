//! The lens runner's [`Model`] port over [`OpenAiHttp`] (#594).
//!
//! Admitted only at zero price (the lens config refuses any other): the
//! adapter forwards the output cap as `max_tokens`, but it cannot bound
//! the input tokens a server's chat template adds, so it gives no
//! before-call cost bound.

use orch_dispatch::{AgentError, ClassifiedError};
use orch_lens::{check, Answer, Model, Question, Shape};

use crate::client::{OpenAiHttp, Resolve};
use crate::wire::{Completion, Prompt, Usage};

impl<R: Resolve> Model for OpenAiHttp<R> {
    /// One chat completion: the question's prefix as the system message,
    /// its body as the user message. The reply must pass [`check`] and
    /// report its usage; the check runs inside the key-redaction boundary.
    fn ask(&mut self, q: &Question) -> Result<Answer, ClassifiedError> {
        let prompt = Prompt {
            system: &q.prefix,
            user: &q.body,
            max_output_tokens: q.max_output_tokens,
        };
        self.complete_then(&prompt, |c| answer(c, &q.shape))
    }
}

/// Check a completion against the question's shape.
///
/// `Permanent`, so nothing unverified reaches the board or the meter, when
/// the reply does not pass [`check`], or when its usage is not a measured
/// count: input or output missing, or more cached tokens than input.
pub fn answer(c: Completion, shape: &Shape) -> Result<Answer, ClassifiedError> {
    let usage = usage(c.usage).map_err(refuse)?;
    let reply = check(&c.text, shape).map_err(refuse)?;
    Ok(Answer { reply, usage })
}

fn refuse(e: String) -> ClassifiedError {
    ClassifiedError::Permanent(AgentError(format!("openai reply: {e}")))
}

/// Measured usage, or why it is not. A missing cached count is zero: the
/// whole input is then charged at the fresh rate, never less.
fn usage(u: Usage) -> Result<orch_lens::Usage, String> {
    let (Some(input), Some(output)) = (u.input_tokens, u.output_tokens) else {
        return Err("usage not reported (prompt_tokens and completion_tokens are required)".into());
    };
    let cached = u.cached_input_tokens.unwrap_or(0);
    if cached > input {
        return Err(format!(
            "usage reports {cached} cached of {input} input tokens"
        ));
    }
    Ok(orch_lens::Usage {
        input_tokens: input,
        output_tokens: output,
        cached_input_tokens: cached,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn none() -> Shape {
        Shape::Posts(orch_lens::Allowed {
            evidence: None,
            claims: None,
        })
    }

    fn completion(
        text: &str,
        input: Option<u64>,
        output: Option<u64>,
        cached: Option<u64>,
    ) -> Completion {
        Completion {
            text: text.to_owned(),
            usage: Usage {
                input_tokens: input,
                output_tokens: output,
                cached_input_tokens: cached,
            },
        }
    }

    const EMPTY: &str = r#"{"posts":[]}"#;

    #[test]
    fn a_conforming_reply_becomes_an_answer_with_usage() {
        let a = answer(completion(EMPTY, Some(40), Some(5), Some(30)), &none()).unwrap();
        assert_eq!(a.reply, orch_lens::Reply::Posts(Vec::new()));
        assert_eq!(
            a.usage,
            orch_lens::Usage {
                input_tokens: 40,
                output_tokens: 5,
                cached_input_tokens: 30,
            }
        );
        let a = answer(completion(EMPTY, Some(40), Some(5), None), &none()).unwrap();
        assert_eq!(a.usage.cached_input_tokens, 0);
    }

    #[test]
    fn a_non_conforming_reply_is_permanent() {
        for text in [
            "not json",
            r#"{"posts":[],"extra":1}"#,
            r#"{"posts":[{"kind":"claim"}]}"#,
        ] {
            let err = answer(completion(text, Some(1), Some(1), None), &none()).unwrap_err();
            assert!(
                matches!(err, ClassifiedError::Permanent(_)),
                "{text}: {err:?}"
            );
        }
    }

    #[test]
    fn unmeasured_usage_is_permanent() {
        for (input, output, cached) in [
            (None, None, None),
            (None, Some(5), None),
            (Some(40), None, None),
            (Some(40), Some(5), Some(41)),
        ] {
            let err = answer(completion(EMPTY, input, output, cached), &none()).unwrap_err();
            assert!(
                matches!(err, ClassifiedError::Permanent(_)),
                "{input:?} {output:?} {cached:?}"
            );
        }
    }
}
