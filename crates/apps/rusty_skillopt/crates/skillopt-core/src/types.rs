use serde::{Deserialize, Serialize};

/// The trainable state: a compact markdown document injected into the
/// executor's context ahead of every rollout.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Skill {
    pub text: String,
}

impl Skill {
    pub fn new(text: impl Into<String>) -> Self {
        Self { text: text.into() }
    }

    pub fn token_estimate(&self) -> usize {
        // Rough heuristic (no tokenizer dependency): ~4 chars/token.
        self.text.chars().count().div_ceil(4)
    }

    pub fn lines(&self) -> Vec<&str> {
        self.text.lines().collect()
    }
}

/// One labeled task instance drawn from a benchmark/environment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Example {
    pub id: String,
    pub input: String,
    pub expected: String,
}

/// A single chat message exchanged with a backend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: content.into(),
        }
    }
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
        }
    }
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
}

/// The record of one rollout: an executor ran an example under a specific
/// skill snapshot and produced an output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trajectory {
    pub example_id: String,
    pub input: String,
    pub expected: String,
    pub output: String,
}

/// Scored + critiqued trajectory, ready for aggregation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reflection {
    pub example_id: String,
    pub score: f64,
    pub critique: String,
}

/// Feedback bundle handed to the optimizer after selection narrows a batch
/// of reflections down to the examples worth spending edit budget on.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AggregatedFeedback {
    pub mean_score: f64,
    pub highlighted: Vec<Reflection>,
    pub rejected_edit_summaries: Vec<String>,
}

/// A single bounded text edit against the skill document. Anchors are exact
/// line matches so edits stay deterministic and auditable rather than
/// depending on line numbers the optimizer can't reliably count.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum EditOp {
    /// Insert `content` immediately after the line matching `anchor`, or at
    /// the end of the document if `anchor` is `None`.
    Add {
        anchor: Option<String>,
        content: String,
    },
    /// Remove the line matching `anchor`.
    Delete { anchor: String },
    /// Replace the line matching `anchor` with `content`.
    Replace { anchor: String, content: String },
}

/// The optimizer's proposed update: a bounded set of ops plus the rationale
/// that gets logged (and fed back on rejection so it isn't retried verbatim).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillEdit {
    pub ops: Vec<EditOp>,
    pub rationale: String,
}

/// Outcome of running a candidate skill against a set of examples.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalResult {
    pub mean_score: f64,
    pub per_example: Vec<(String, f64)>,
}

impl EvalResult {
    pub fn from_scores(scores: Vec<(String, f64)>) -> Self {
        let mean_score = if scores.is_empty() {
            0.0
        } else {
            scores.iter().map(|(_, s)| s).sum::<f64>() / scores.len() as f64
        };
        Self {
            mean_score,
            per_example: scores,
        }
    }
}

/// One accepted or rejected step, logged for the training report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepRecord {
    pub epoch: usize,
    pub batch: usize,
    pub accepted: bool,
    pub rationale: String,
    pub train_mean_score: f64,
    pub val_mean_score: f64,
    pub best_val_mean_score: f64,
}
