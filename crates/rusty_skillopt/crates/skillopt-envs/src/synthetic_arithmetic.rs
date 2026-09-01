use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use serde::{Deserialize, Serialize};
use skillopt_core::{Environment, Example};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SyntheticArithmeticParams {
    pub train_size: usize,
    pub val_size: usize,
    pub test_size: usize,
    pub seed: u64,
    /// Per-distractor inclusion probability (0.0-1.0). Each distractor is
    /// an irrelevant sentence, e.g. "Alice is 12 years old." — a number
    /// that shouldn't affect the answer. This is where a naive initial
    /// skill tends to fail and gives the optimizer something real to fix.
    pub distractor_rate: f64,
    /// Max number of distractor sentences considered per problem (each
    /// included independently with probability `distractor_rate`).
    pub max_distractors: usize,
    /// Fraction of problems (0.0-1.0) that chain 2-3 sequential operations
    /// (gain/lose/double/halve) instead of a single one. Tracking a running
    /// total across several steps — in the stated order, ignoring
    /// distractors along the way — is where small models start making real
    /// mistakes, which is exactly the kind of failure a skill instruction
    /// like "recompute the running total after each step" can fix.
    pub multi_step_rate: f64,
}

impl Default for SyntheticArithmeticParams {
    fn default() -> Self {
        Self {
            train_size: 24,
            val_size: 8,
            test_size: 16,
            seed: 0,
            distractor_rate: 0.5,
            max_distractors: 1,
            multi_step_rate: 0.0,
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum StepKind {
    Gain(i64),
    Lose(i64),
    Double,
    Halve,
}

/// A small, fully offline, deterministically-generated word-problem
/// benchmark. Standing in for SkillOpt's real benchmark adapters (e.g.
/// SearchQA): programmatic scoring means the validation gate never itself
/// depends on an LLM call, and generation needs no network or dataset file.
pub struct SyntheticArithmeticEnv {
    train: Vec<Example>,
    val: Vec<Example>,
    test: Vec<Example>,
}

impl SyntheticArithmeticEnv {
    pub fn new(params: SyntheticArithmeticParams) -> Self {
        let mut rng = StdRng::seed_from_u64(params.seed);
        let mut next_id = 0usize;
        let mut gen = |rng: &mut StdRng, n: usize, prefix: &str| -> Vec<Example> {
            (0..n)
                .map(|_| {
                    let ex = gen_example(rng, &params, next_id, prefix);
                    next_id += 1;
                    ex
                })
                .collect()
        };
        let train = gen(&mut rng, params.train_size, "train");
        let val = gen(&mut rng, params.val_size, "val");
        let test = gen(&mut rng, params.test_size, "test");
        Self { train, val, test }
    }
}

fn gen_example(
    rng: &mut StdRng,
    params: &SyntheticArithmeticParams,
    id: usize,
    prefix: &str,
) -> Example {
    let names = ["Alice", "Bob", "Carla", "Dev", "Ewa", "Femi"];
    let items = ["apples", "marbles", "stickers", "coins", "books"];

    let name = names[rng.gen_range(0..names.len())];
    let item = items[rng.gen_range(0..items.len())];
    let start: i64 = rng.gen_range(1..30);

    let mut value = start;
    let mut sentences = vec![format!("{name} has {value} {item}.")];

    let multi_step = rng.gen_bool(params.multi_step_rate);
    let steps = if multi_step { rng.gen_range(2..=3) } else { 1 };

    for _ in 0..steps {
        let mut candidates: Vec<StepKind> = vec![StepKind::Gain(rng.gen_range(1..12))];
        if value >= 1 {
            candidates.push(StepKind::Lose(rng.gen_range(1..=value.min(12))));
        }
        // Double/halve only show up in multi-step chains, so single-op
        // problems stay plain addition/subtraction like before.
        if multi_step && (1..=40).contains(&value) {
            candidates.push(StepKind::Double);
        }
        if multi_step && value % 2 == 0 && value >= 2 {
            candidates.push(StepKind::Halve);
        }

        let step = candidates[rng.gen_range(0..candidates.len())];
        let sentence = match step {
            StepKind::Gain(d) => {
                value += d;
                format!("Then {name} gets {d} more {item}.")
            }
            StepKind::Lose(d) => {
                value -= d;
                format!("Then {name} gives away {d} {item}.")
            }
            StepKind::Double => {
                value *= 2;
                format!("Then {name}'s {item} collection doubles.")
            }
            StepKind::Halve => {
                value /= 2;
                format!("Then {name} gives away half of the {item}.")
            }
        };
        sentences.push(sentence);
    }

    // Excludes the protagonist so a distractor can never restate/contradict
    // their own count (e.g. "Bob has 18 stickers... Bob has 1 stickers.").
    let other_names: Vec<&str> = names.into_iter().filter(|n| *n != name).collect();

    for _ in 0..params.max_distractors {
        if rng.gen_bool(params.distractor_rate) {
            let distractor_name = other_names[rng.gen_range(0..other_names.len())];
            let distractor = if rng.gen_bool(0.5) {
                let age: i64 = rng.gen_range(5..80);
                format!("{distractor_name} is {age} years old.")
            } else {
                let other_item = items[rng.gen_range(0..items.len())];
                let count: i64 = rng.gen_range(1..20);
                format!("{distractor_name} has {count} {other_item}.")
            };
            sentences.push(distractor);
        }
    }

    sentences.push(format!("How many {item} does {name} have now?"));

    Example {
        id: format!("{prefix}-{id}"),
        input: sentences.join(" "),
        expected: value.to_string(),
    }
}

impl Environment for SyntheticArithmeticEnv {
    fn name(&self) -> &str {
        "synthetic_arithmetic"
    }
    fn train_examples(&self) -> &[Example] {
        &self.train
    }
    fn val_examples(&self) -> &[Example] {
        &self.val
    }
    fn test_examples(&self) -> &[Example] {
        &self.test
    }

    /// Lenient-but-strict scoring: extracts the *last* integer literal in
    /// the output (tolerating surrounding prose like "The answer is 7.")
    /// and compares it exactly to the expected integer.
    fn score(&self, example: &Example, output: &str) -> f64 {
        let expected: i64 = match example.expected.parse() {
            Ok(v) => v,
            Err(_) => return 0.0,
        };
        match last_integer(output) {
            Some(actual) if actual == expected => 1.0,
            _ => 0.0,
        }
    }
}

fn last_integer(text: &str) -> Option<i64> {
    let mut found: Option<i64> = None;
    let bytes: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < bytes.len() {
        let negative = bytes[i] == '-' && i + 1 < bytes.len() && bytes[i + 1].is_ascii_digit();
        let start = if negative { i + 1 } else { i };
        if bytes.get(start).is_some_and(|c| c.is_ascii_digit()) {
            let mut j = start;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
            let digits: String = bytes[start..j].iter().collect();
            if let Ok(mut v) = digits.parse::<i64>() {
                if negative {
                    v = -v;
                }
                found = Some(v);
            }
            i = j;
        } else {
            i += 1;
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distractors_never_restate_the_protagonist() {
        // Regression test: a distractor sentence used to be able to pick the
        // same name as the protagonist (and even the same item), producing
        // self-contradictory problems like "Bob has 18 stickers... Bob has 1
        // stickers." — confusing the model for reasons unrelated to the
        // skill being tested. Forced distractor_rate: 1.0 to exercise every
        // possible distractor slot across a large batch.
        let env = SyntheticArithmeticEnv::new(SyntheticArithmeticParams {
            train_size: 200,
            val_size: 0,
            test_size: 0,
            seed: 7,
            distractor_rate: 1.0,
            max_distractors: 2,
            multi_step_rate: 0.5,
        });
        for e in env.train_examples() {
            let protagonist = e.input.split(" has ").next().unwrap();
            let protagonist_restated = format!("{protagonist} has ");
            let occurrences = e.input.matches(&protagonist_restated).count();
            assert_eq!(
                occurrences, 1,
                "protagonist should only be introduced once, found in: {}",
                e.input
            );
        }
    }

    #[test]
    fn generation_is_deterministic_given_seed() {
        let a = SyntheticArithmeticEnv::new(SyntheticArithmeticParams {
            seed: 5,
            ..Default::default()
        });
        let b = SyntheticArithmeticEnv::new(SyntheticArithmeticParams {
            seed: 5,
            ..Default::default()
        });
        assert_eq!(
            a.train.iter().map(|e| &e.input).collect::<Vec<_>>(),
            b.train.iter().map(|e| &e.input).collect::<Vec<_>>()
        );
    }

    #[test]
    fn splits_have_requested_sizes_and_no_overlap() {
        let env = SyntheticArithmeticEnv::new(SyntheticArithmeticParams {
            train_size: 5,
            val_size: 2,
            test_size: 3,
            seed: 1,
            distractor_rate: 0.5,
            ..Default::default()
        });
        assert_eq!(env.train_examples().len(), 5);
        assert_eq!(env.val_examples().len(), 2);
        assert_eq!(env.test_examples().len(), 3);
        let mut ids: Vec<&str> = env
            .train_examples()
            .iter()
            .chain(env.val_examples())
            .chain(env.test_examples())
            .map(|e| e.id.as_str())
            .collect();
        let before = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(
            ids.len(),
            before,
            "example ids must be unique across splits"
        );
    }

    #[test]
    fn scores_exact_answer_amid_prose() {
        let env = SyntheticArithmeticEnv::new(SyntheticArithmeticParams::default());
        let ex = Example {
            id: "x".into(),
            input: String::new(),
            expected: "7".into(),
        };
        assert_eq!(env.score(&ex, "The answer is 7."), 1.0);
        assert_eq!(env.score(&ex, "7"), 1.0);
        assert_eq!(env.score(&ex, "I think it's 8, definitely 8."), 0.0);
        assert_eq!(env.score(&ex, "no numbers here"), 0.0);
    }

    #[test]
    fn scores_negative_answers() {
        let env = SyntheticArithmeticEnv::new(SyntheticArithmeticParams::default());
        let ex = Example {
            id: "x".into(),
            input: String::new(),
            expected: "-3".into(),
        };
        assert_eq!(env.score(&ex, "The result is -3."), 1.0);
        assert_eq!(env.score(&ex, "The result is 3."), 0.0);
    }

    #[test]
    fn multi_step_problems_self_score_and_stay_nonnegative() {
        let env = SyntheticArithmeticEnv::new(SyntheticArithmeticParams {
            train_size: 40,
            val_size: 10,
            test_size: 10,
            seed: 42,
            distractor_rate: 1.0,
            max_distractors: 2,
            multi_step_rate: 1.0,
        });
        let mut saw_multi_sentence = false;
        for e in env.train_examples() {
            let expected: i64 = e.expected.parse().unwrap();
            assert!(expected >= 0, "value should never go negative: {}", e.input);
            assert_eq!(env.score(e, &e.expected), 1.0);
            if e.input.matches("Then ").count() >= 2 {
                saw_multi_sentence = true;
            }
        }
        assert!(
            saw_multi_sentence,
            "expected at least one multi-step problem in the batch"
        );
    }

    #[test]
    fn expected_answers_match_generated_arithmetic() {
        let env = SyntheticArithmeticEnv::new(SyntheticArithmeticParams {
            seed: 3,
            ..Default::default()
        });
        for e in env.train_examples() {
            // Every generated expected value must itself be parseable and
            // recoverable via the same scoring function (sanity check that
            // the scorer and generator agree).
            let expected: i64 = e.expected.parse().unwrap();
            assert_eq!(
                env.score(e, &e.expected),
                1.0,
                "expected {expected} should self-score 1.0"
            );
        }
    }
}
