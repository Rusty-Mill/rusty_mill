//! The AIDE0 search: a tree of attempts and the policy that grows it.

use std::io;

use crate::broker::{Broker, Reply};
use crate::prompt;
use crate::rng::Rng;

/// Drafts written before the agent starts improving or debugging.
pub const DRAFTS: usize = 5;
/// Probability of debugging, when a buggy leaf is available.
pub const DEBUG_PROBABILITY: f64 = 0.5;
/// Buggy nodes deeper than this in a debug chain are abandoned.
pub const MAX_DEBUG_DEPTH: usize = 3;

/// How a node was produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// A fresh solution.
    Draft,
    /// A change to the given (best) node.
    Improve(usize),
    /// A fix for the given buggy node.
    Debug(usize),
}

impl Step {
    const fn parent(self) -> Option<usize> {
        match self {
            Self::Draft => None,
            Self::Improve(parent) | Self::Debug(parent) => Some(parent),
        }
    }
}

/// One attempt.
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    /// How it was produced.
    pub step: Step,
    /// The extracted program, if the reply contained one.
    pub code: Option<String>,
    /// The public score; `None` means buggy.
    pub score: Option<f64>,
    /// The run's output, or why there was no run.
    pub feedback: String,
    /// Consecutive debug steps leading to this node.
    pub debug_depth: usize,
}

/// Every attempt so far, in order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Tree {
    /// The attempts; a node's index is its id.
    pub nodes: Vec<Node>,
}

impl Tree {
    /// The best-scoring node; the earliest one on ties.
    pub fn best(&self) -> Option<usize> {
        let mut best: Option<(usize, f64)> = None;
        for (index, node) in self.nodes.iter().enumerate() {
            if let Some(score) = node.score {
                if best.is_none_or(|(_, top)| score > top) {
                    best = Some((index, score));
                }
            }
        }
        best.map(|(index, _)| index)
    }

    /// Buggy leaves still worth debugging.
    fn debuggable(&self) -> Vec<usize> {
        let has_child = |index: usize| self.nodes.iter().any(|n| n.step.parent() == Some(index));
        (0..self.nodes.len())
            .filter(|&i| {
                let node = &self.nodes[i];
                node.score.is_none() && node.debug_depth <= MAX_DEBUG_DEPTH && !has_child(i)
            })
            .collect()
    }

    /// AIDE0's policy: draft, then debug or improve greedily.
    pub fn choose(&self, rng: &mut Rng) -> Step {
        let drafts = self.nodes.iter().filter(|n| n.step == Step::Draft).count();
        if drafts < DRAFTS {
            return Step::Draft;
        }
        if rng.chance(DEBUG_PROBABILITY) {
            let buggy = self.debuggable();
            if !buggy.is_empty() {
                return Step::Debug(buggy[rng.below(buggy.len())]);
            }
        }
        self.best().map_or(Step::Draft, Step::Improve)
    }

    /// Appends a node; returns its id.
    pub fn push(
        &mut self,
        step: Step,
        code: Option<String>,
        score: Option<f64>,
        feedback: String,
    ) -> usize {
        let debug_depth = match step {
            Step::Debug(parent) => self.nodes[parent].debug_depth + 1,
            Step::Draft | Step::Improve(_) => 0,
        };
        self.nodes.push(Node {
            step,
            code,
            score,
            feedback,
            debug_depth,
        });
        self.nodes.len() - 1
    }
}

/// The agent: a task, a seed and the search tree.
#[derive(Debug)]
pub struct Agent {
    task: String,
    rng: Rng,
    tree: Tree,
}

impl Agent {
    /// An agent for the task described by `task`.
    pub const fn new(task: String, seed: u64) -> Self {
        Self {
            task,
            rng: Rng::new(seed),
            tree: Tree { nodes: Vec::new() },
        }
    }

    /// Searches until the budget is spent, submitting each new best.
    ///
    /// # Errors
    /// When the broker hangs up or answers something unexpected.
    pub fn run(&mut self, broker: &mut impl Broker) -> io::Result<()> {
        loop {
            let step = self.tree.choose(&mut self.rng);
            let messages = prompt::messages(&self.task, &self.tree, step);
            let reply = match broker.llm(&messages)? {
                Reply::Text(text) => text,
                Reply::Exhausted(_) => return Ok(()),
                other => return Err(unexpected(&other)),
            };
            let code = prompt::extract_code(&reply);
            let (score, feedback) = match &code {
                None => (None, prompt::NO_CODE.to_owned()),
                Some(source) => match broker.eval(source)? {
                    Reply::Evaluated { score, feedback } => (score, feedback),
                    Reply::Exhausted(_) => return Ok(()),
                    Reply::Refused(reason) => (None, format!("The broker refused it: {reason}")),
                    other => return Err(unexpected(&other)),
                },
            };
            let index = self.tree.push(step, code, score, feedback);
            if self.tree.best() == Some(index) {
                if let Some(source) = &self.tree.nodes[index].code {
                    broker.submit(source)?;
                }
            }
        }
    }
}

fn unexpected(reply: &Reply) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("unexpected reply {reply:?}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::broker::Message;

    fn node(tree: &mut Tree, step: Step, score: Option<f64>) -> usize {
        tree.push(step, Some("pass".to_owned()), score, String::new())
    }

    #[test]
    fn drafts_first_then_improves_the_best() {
        let mut tree = Tree::default();
        let mut rng = Rng::new(1);
        for score in [Some(0.2), Some(0.7), Some(0.7), Some(0.1)] {
            assert_eq!(tree.choose(&mut rng), Step::Draft);
            node(&mut tree, Step::Draft, score);
        }
        assert_eq!(tree.choose(&mut rng), Step::Draft, "four drafts so far");
        node(&mut tree, Step::Draft, Some(0.3));
        for _ in 0..50 {
            assert_eq!(tree.choose(&mut rng), Step::Improve(1), "earliest best");
        }
    }

    #[test]
    fn debugs_only_buggy_leaves_within_the_depth_limit() {
        let mut tree = Tree::default();
        node(&mut tree, Step::Draft, Some(0.5));
        let mut chain = node(&mut tree, Step::Draft, None);
        for _ in 0..3 {
            node(&mut tree, Step::Draft, Some(0.1));
        }
        let mut rng = Rng::new(3);
        let mut saw_debug = false;
        for _ in 0..200 {
            match tree.choose(&mut rng) {
                Step::Debug(target) => {
                    assert_eq!(target, chain);
                    saw_debug = true;
                }
                step => assert_eq!(step, Step::Improve(0)),
            }
        }
        assert!(saw_debug);
        for depth in 1..=MAX_DEBUG_DEPTH + 1 {
            chain = node(&mut tree, Step::Debug(chain), None);
            assert_eq!(tree.nodes[chain].debug_depth, depth);
        }
        assert!(
            (0..200).all(|_| tree.choose(&mut rng) == Step::Improve(0)),
            "depth {} is past the limit",
            MAX_DEBUG_DEPTH + 1
        );
    }

    /// Answers from a script and records what the agent asked.
    struct Scripted {
        replies: Vec<&'static str>,
        scores: Vec<Option<f64>>,
        llm_budget: usize,
        submitted: Vec<String>,
        prompts: Vec<Vec<Message>>,
    }

    impl Broker for Scripted {
        fn llm(&mut self, messages: &[Message]) -> io::Result<Reply> {
            if self.prompts.len() == self.llm_budget {
                return Ok(Reply::Exhausted("tokens".to_owned()));
            }
            self.prompts.push(messages.to_vec());
            let reply = self.replies[(self.prompts.len() - 1) % self.replies.len()];
            Ok(Reply::Text(reply.to_owned()))
        }

        fn eval(&mut self, _: &str) -> io::Result<Reply> {
            Ok(Reply::Evaluated {
                score: self.scores.remove(0),
                feedback: "ran".to_owned(),
            })
        }

        fn submit(&mut self, source: &str) -> io::Result<Reply> {
            self.submitted.push(source.to_owned());
            Ok(Reply::Submitted)
        }
    }

    #[test]
    fn submits_each_new_best_and_stops_when_the_budget_is_spent() {
        let mut broker = Scripted {
            replies: vec![
                "```python\nprint(1)\n```",
                "no code here",
                "```python\nprint(2)\n```",
            ],
            scores: vec![Some(0.3), Some(0.6), Some(0.6), Some(0.9)],
            llm_budget: 6,
            submitted: Vec::new(),
            prompts: Vec::new(),
        };
        Agent::new("task".to_owned(), 9)
            .run(&mut broker)
            .expect("runs to the budget");
        assert_eq!(broker.prompts.len(), 6);
        assert!(broker.scores.is_empty(), "every program ran; prose did not");
        assert_eq!(
            broker.submitted,
            vec!["print(1)\n", "print(2)\n", "print(2)\n"],
            "0.3, then 0.6, then 0.9; the tie at 0.6 is not a new best"
        );
        assert!(broker.prompts[0][0].content.contains("task"));
    }

    #[test]
    fn identical_responses_give_identical_requests() {
        let run = || {
            let mut broker = Scripted {
                replies: vec!["```python\nprint(1)\n```"],
                scores: [None, Some(0.2)].repeat(10),
                llm_budget: 20,
                submitted: Vec::new(),
                prompts: Vec::new(),
            };
            Agent::new("task".to_owned(), 5)
                .run(&mut broker)
                .expect("runs");
            broker.prompts
        };
        assert_eq!(run(), run());
    }
}
