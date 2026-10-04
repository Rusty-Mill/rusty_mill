//! The broker: the inner agent's only way out (ADR-0005 §3, invariant 2).
//!
//! [`serve`] answers the agent's requests over its socket and records every
//! exchange. What answers them is a [`Service`]:
//!
//! - [`LiveService`] calls the model and the public scorer, metering both
//!   against the run's [`Budget`]. Once the budget is spent it answers
//!   everything but `submit` with `exhausted`.
//! - [`ReplayService`] answers from a recorded transcript instead and fails
//!   as soon as the agent asks for something the recording did not. That
//!   is trajectory replay: a deterministic agent must reproduce its run.

use std::io::{Read, Write};
use std::time::Instant;

use rsi_core::{Budget, ChatModel, CostMeter, CostUsage, Message, PublicTask, Seed, Solution};

use crate::error::RuntimeError;
use crate::protocol::{read_frame, write_frame, Exchange, Request, Response};

/// Answers agent requests.
pub trait Service {
    /// The response to `request`, or `None` to end the session.
    ///
    /// # Errors
    /// Infrastructure failures, which end the run: the model is down, the
    /// sandbox failed, or a replay diverged.
    fn handle(&mut self, request: &Request) -> Result<Option<Response>, RuntimeError>;
}

/// Serves one agent session on `stream` until the agent hangs up, the
/// service ends it, or the agent sends a malformed frame. Returns every
/// exchange, including one whose response could not be delivered because
/// the agent had already died.
///
/// # Errors
/// What [`Service::handle`] returns. A misbehaving agent is not an error:
/// its session simply ends.
pub fn serve<S: Read + Write>(
    mut stream: S,
    service: &mut impl Service,
) -> Result<Vec<Exchange>, RuntimeError> {
    let mut exchanges = Vec::new();
    loop {
        // Anything wrong on the agent's side of the wire ends its session;
        // the agent is untrusted, and its run is graded on what it did.
        let Ok(Some(frame)) = read_frame(&mut stream) else {
            return Ok(exchanges);
        };
        let Ok(request) = Request::decode(&frame) else {
            return Ok(exchanges);
        };
        let Some(response) = service.handle(&request)? else {
            return Ok(exchanges);
        };
        let delivered = write_frame(&mut stream, &response.encode());
        exchanges.push(Exchange { request, response });
        if delivered.is_err() {
            return Ok(exchanges);
        }
    }
}

/// The agent's last accepted submission (`x̂_t`).
#[must_use]
pub fn final_submission(exchanges: &[Exchange]) -> Option<Solution> {
    exchanges
        .iter()
        .rev()
        .find_map(|exchange| match (&exchange.request, &exchange.response) {
            (Request::Submit(source), Response::Submitted) => Solution::new(source.clone()).ok(),
            _ => None,
        })
}

/// Serves a live run: the real model and the task's public scorer.
#[derive(Debug)]
pub struct LiveService<'a, M, T> {
    model: &'a M,
    task: &'a T,
    seed: Seed,
    meter: CostMeter,
    started: Instant,
    max_completion_tokens: u64,
}

impl<'a, M, T> LiveService<'a, M, T> {
    /// A service for one run of `task` within `budget`. Every model call
    /// asks for at most `max_completion_tokens`, and never more than the
    /// tokens left. The wall clock starts now.
    #[must_use]
    pub fn new(
        model: &'a M,
        task: &'a T,
        budget: Budget,
        seed: Seed,
        max_completion_tokens: u64,
    ) -> Self {
        Self {
            model,
            task,
            seed,
            meter: CostMeter::new(budget),
            started: Instant::now(),
            max_completion_tokens,
        }
    }

    /// Usage so far, with the wall clock read now.
    #[must_use]
    pub fn usage(&mut self) -> CostUsage {
        self.meter.observe_wall(self.started.elapsed());
        *self.meter.usage()
    }
}

impl<M, T> LiveService<'_, M, T>
where
    M: ChatModel,
    M::Error: std::fmt::Display,
    T: PublicTask<Error = RuntimeError>,
{
    /// Runs `op` if the budget admits it, charging the wall clock around it.
    fn metered(
        &mut self,
        op: impl FnOnce(&mut Self) -> Result<Response, RuntimeError>,
    ) -> Result<Response, RuntimeError> {
        self.meter.observe_wall(self.started.elapsed());
        if let Err(exhausted) = self.meter.admit() {
            return Ok(Response::Exhausted(exhausted.to_string()));
        }
        let response = op(self)?;
        self.meter.observe_wall(self.started.elapsed());
        Ok(response)
    }

    fn llm(&mut self, messages: &[Message]) -> Result<Response, RuntimeError> {
        let max_tokens = self
            .max_completion_tokens
            .min(self.meter.remaining_tokens());
        let completion = self
            .model
            .complete(messages, max_tokens)
            .map_err(|e| RuntimeError::Model(e.to_string()))?;
        self.meter
            .record_tokens(completion.prompt_tokens, completion.completion_tokens);
        Ok(Response::Text(completion.text))
    }

    fn eval(&self, source: &str) -> Result<Response, RuntimeError> {
        let solution = match Solution::new(source) {
            Ok(solution) => solution,
            Err(error) => return Ok(Response::Refused(error.to_string())),
        };
        let attempt = self.task.public_score(&solution, self.seed)?;
        Ok(Response::Evaluated {
            score: attempt.score,
            feedback: attempt.feedback,
        })
    }
}

impl<M, T> Service for LiveService<'_, M, T>
where
    M: ChatModel,
    M::Error: std::fmt::Display,
    T: PublicTask<Error = RuntimeError>,
{
    fn handle(&mut self, request: &Request) -> Result<Option<Response>, RuntimeError> {
        let response = match request {
            // Submitting is free and stays open after the budget is spent.
            Request::Submit(source) => match Solution::new(source.as_str()) {
                Ok(_) => Response::Submitted,
                Err(error) => Response::Refused(error.to_string()),
            },
            Request::Llm(messages) => self.metered(|service| service.llm(messages))?,
            Request::Eval(source) => self.metered(|service| service.eval(source))?,
        };
        Ok(Some(response))
    }
}

/// Replays a recorded session.
#[derive(Debug)]
pub struct ReplayService {
    recorded: Vec<Exchange>,
    next: usize,
}

impl ReplayService {
    /// A service that answers from `recorded`, in order.
    #[must_use]
    pub const fn new(recorded: Vec<Exchange>) -> Self {
        Self { recorded, next: 0 }
    }

    /// Fails unless every recorded exchange was replayed.
    ///
    /// # Errors
    /// [`RuntimeError::Broker`] when the agent stopped early.
    pub fn finish(&self) -> Result<(), RuntimeError> {
        if self.next == self.recorded.len() {
            return Ok(());
        }
        Err(RuntimeError::Broker(format!(
            "replay diverged: the agent stopped after {} of {} exchanges",
            self.next,
            self.recorded.len()
        )))
    }
}

impl Service for ReplayService {
    fn handle(&mut self, request: &Request) -> Result<Option<Response>, RuntimeError> {
        let Some(recorded) = self.recorded.get(self.next) else {
            // The recording ends here (the live agent exited or was killed):
            // hang up, exactly as the live session ended.
            return Ok(None);
        };
        if &recorded.request != request {
            return Err(RuntimeError::Broker(format!(
                "replay diverged at exchange {}: the agent sent a different request",
                self.next
            )));
        }
        self.next += 1;
        Ok(Some(recorded.response.clone()))
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::time::Duration;

    use rsi_core::{Attempt, Completion, ModelId, Role, Score, TaskId};

    use super::*;
    use crate::protocol::{encode_transcript, MAX_FRAME_BYTES};

    struct Model {
        id: ModelId,
        tokens: (u64, u64),
        asked: Cell<Vec<u64>>,
    }

    impl ChatModel for Model {
        type Error = String;

        fn id(&self) -> &ModelId {
            &self.id
        }

        fn complete(&self, _: &[Message], max_tokens: u64) -> Result<Completion, String> {
            let mut asked = self.asked.take();
            asked.push(max_tokens);
            self.asked.set(asked);
            Ok(Completion {
                text: "reply".into(),
                prompt_tokens: self.tokens.0,
                completion_tokens: self.tokens.1,
            })
        }
    }

    struct Task {
        id: TaskId,
        baseline: Solution,
    }

    impl PublicTask for Task {
        type Error = RuntimeError;

        fn id(&self) -> &TaskId {
            &self.id
        }

        fn baseline(&self) -> &Solution {
            &self.baseline
        }

        fn description(&self) -> &str {
            "toy"
        }

        fn public_score(&self, solution: &Solution, _: Seed) -> Result<Attempt, RuntimeError> {
            let score = solution.source().contains("good").then_some(Score::ONE);
            Ok(Attempt {
                score,
                feedback: "ran".into(),
                wall: Duration::ZERO,
            })
        }
    }

    fn model(prompt: u64, completion: u64) -> Model {
        Model {
            id: ModelId::parse("scripted").expect("valid"),
            tokens: (prompt, completion),
            asked: Cell::new(Vec::new()),
        }
    }

    fn task() -> Task {
        Task {
            id: TaskId::parse("toy").expect("valid"),
            baseline: Solution::new("pass").expect("valid"),
        }
    }

    fn llm() -> Request {
        Request::Llm(vec![Message {
            role: Role::User,
            content: "go".into(),
        }])
    }

    fn budget(tokens: u64) -> Budget {
        Budget::new(tokens, Duration::from_secs(60), None).expect("valid")
    }

    /// An in-memory agent: scripted requests in, responses out.
    struct Wire {
        input: std::io::Cursor<Vec<u8>>,
        output: Vec<u8>,
    }

    impl Wire {
        fn new(requests: &[Request]) -> Self {
            let mut input = Vec::new();
            for request in requests {
                write_frame(&mut input, &request.encode()).expect("frame");
            }
            Self {
                input: std::io::Cursor::new(input),
                output: Vec::new(),
            }
        }
    }

    impl Read for Wire {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.input.read(buf)
        }
    }

    impl Write for Wire {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.output.write(buf)
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn the_budget_is_a_hard_stop_except_for_submit() {
        let (model, task) = (model(30, 20), task());
        let mut service = LiveService::new(&model, &task, budget(120), Seed::new(1), 64);
        let requests = [
            llm(),
            Request::Eval("good".into()),
            llm(),
            llm(),
            Request::Eval("good".into()),
            llm(),
            Request::Submit("good".into()),
        ];
        let exchanges = serve(Wire::new(&requests), &mut service).expect("serves");
        let responses: Vec<_> = exchanges.iter().map(|e| e.response.clone()).collect();
        assert!(matches!(responses[0], Response::Text(_)));
        assert!(matches!(
            responses[1],
            Response::Evaluated { score: Some(_), .. }
        ));
        assert!(matches!(responses[2], Response::Text(_)), "50 of 120 used");
        assert!(matches!(responses[3], Response::Text(_)), "100 of 120 used");
        assert!(matches!(responses[4], Response::Exhausted(_)), "150 >= 120");
        assert!(matches!(responses[5], Response::Exhausted(_)));
        assert_eq!(responses[6], Response::Submitted);
        assert_eq!(service.usage().tokens(), 150, "the overshoot is recorded");
        assert_eq!(
            model.asked.take(),
            vec![64, 64, 20],
            "never more than the cap or the tokens left"
        );
        assert_eq!(
            final_submission(&exchanges).map(|s| s.source().to_owned()),
            Some("good".into())
        );
    }

    #[test]
    fn a_completion_never_asks_for_more_than_the_tokens_left() {
        let (model, task) = (model(10, 10), task());
        let mut service = LiveService::new(&model, &task, budget(25), Seed::new(1), 1000);
        serve(Wire::new(&[llm(), llm()]), &mut service).expect("serves");
        assert_eq!(model.asked.take(), vec![25, 5]);
    }

    #[test]
    fn invalid_solutions_are_refused_and_the_session_continues() {
        let (model, task) = (model(1, 1), task());
        let mut service = LiveService::new(&model, &task, budget(100), Seed::new(1), 8);
        let requests = [
            Request::Eval(" ".into()),
            Request::Submit("a\0b".into()),
            Request::Submit("good".into()),
            Request::Submit("bad".into()),
        ];
        let exchanges = serve(Wire::new(&requests), &mut service).expect("serves");
        assert!(matches!(exchanges[0].response, Response::Refused(_)));
        assert!(matches!(exchanges[1].response, Response::Refused(_)));
        assert_eq!(exchanges.len(), 4);
        assert_eq!(
            final_submission(&exchanges).map(|s| s.source().to_owned()),
            Some("bad".into()),
            "the latest valid submission wins"
        );
    }

    #[test]
    fn a_malformed_frame_ends_the_session_quietly() {
        let (model, task) = (model(1, 1), task());
        let mut service = LiveService::new(&model, &task, budget(100), Seed::new(1), 8);
        let mut wire = Wire::new(&[Request::Submit("good".into())]);
        let mut input = wire.input.into_inner();
        write_frame(&mut input, b"\x09junk").expect("frame");
        input.extend_from_slice(&(MAX_FRAME_BYTES as u32 + 1).to_be_bytes());
        wire.input = std::io::Cursor::new(input);
        let exchanges = serve(wire, &mut service).expect("not an error");
        assert_eq!(exchanges.len(), 1);
    }

    #[test]
    fn replay_reproduces_responses_and_detects_divergence() {
        let (model, task) = (model(5, 5), task());
        let requests = [
            llm(),
            Request::Eval("good".into()),
            Request::Submit("good".into()),
        ];
        let mut live = LiveService::new(&model, &task, budget(100), Seed::new(1), 8);
        let recorded = serve(Wire::new(&requests), &mut live).expect("serves");
        let bytes = encode_transcript(&recorded).expect("encodes");

        let mut replay = ReplayService::new(recorded.clone());
        let replayed = serve(Wire::new(&requests), &mut replay).expect("replays");
        assert_eq!(replayed, recorded);
        replay.finish().expect("complete");
        assert_eq!(encode_transcript(&replayed).expect("encodes"), bytes);

        let mut diverging = ReplayService::new(recorded.clone());
        let other = [llm(), Request::Eval("other".into())];
        assert!(matches!(
            serve(Wire::new(&other), &mut diverging),
            Err(RuntimeError::Broker(_))
        ));

        let mut early = ReplayService::new(recorded.clone());
        serve(Wire::new(&requests[..1]), &mut early).expect("serves");
        assert!(early.finish().is_err(), "stopped early");

        let mut longer = ReplayService::new(recorded);
        let extra = [requests.as_slice(), &[llm()]].concat();
        let replayed = serve(Wire::new(&extra), &mut longer).expect("hangs up");
        assert_eq!(replayed.len(), 3, "the recording's end ends the session");
        longer.finish().expect("complete");
    }
}
