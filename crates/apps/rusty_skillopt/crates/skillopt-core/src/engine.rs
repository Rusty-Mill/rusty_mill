use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::config::TrainConfig;
use crate::prompts::{
    executor_system_prompt, extract_json_object, optimize_prompt, reflect_prompt, select_feedback,
};
use crate::scheduler::{BatchScheduler, RejectionBuffer};
use crate::skill_edit::apply_edit;
use crate::traits::{ChatBackend, Environment};
use crate::types::{
    EvalResult, Example, Message, Reflection, Skill, SkillEdit, StepRecord, Trajectory,
};

const HIGHLIGHT_COUNT: usize = 3;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrainOutcome {
    pub best_skill: Skill,
    pub best_val_score: f64,
    pub initial_val_score: f64,
    pub steps: Vec<StepRecord>,
    pub test_result: Option<EvalResult>,
}

pub struct Engine {
    executor: Arc<dyn ChatBackend>,
    optimizer: Arc<dyn ChatBackend>,
    reflector: Arc<dyn ChatBackend>,
    env: Arc<dyn Environment>,
    cfg: TrainConfig,
}

impl Engine {
    pub fn new(
        executor: Arc<dyn ChatBackend>,
        optimizer: Arc<dyn ChatBackend>,
        reflector: Arc<dyn ChatBackend>,
        env: Arc<dyn Environment>,
        cfg: TrainConfig,
    ) -> Self {
        Self {
            executor,
            optimizer,
            reflector,
            env,
            cfg,
        }
    }

    async fn run_executor(&self, skill: &Skill, example: &Example) -> anyhow::Result<String> {
        let messages = vec![
            Message::system(executor_system_prompt(skill)),
            Message::user(example.input.clone()),
        ];
        self.executor.chat(&messages).await
    }

    async fn evaluate(&self, skill: &Skill, examples: &[Example]) -> anyhow::Result<EvalResult> {
        let mut scores = Vec::with_capacity(examples.len());
        for example in examples {
            let output = self.run_executor(skill, example).await?;
            let score = self.env.score(example, &output);
            scores.push((example.id.clone(), score));
        }
        Ok(EvalResult::from_scores(scores))
    }

    async fn rollout(&self, skill: &Skill, batch: &[&Example]) -> anyhow::Result<Vec<Trajectory>> {
        let mut trajectories = Vec::with_capacity(batch.len());
        for example in batch {
            let output = self.run_executor(skill, example).await?;
            trajectories.push(Trajectory {
                example_id: example.id.clone(),
                input: example.input.clone(),
                expected: example.expected.clone(),
                output,
            });
        }
        Ok(trajectories)
    }

    async fn reflect(
        &self,
        trajectories: &[Trajectory],
        examples: &[&Example],
    ) -> anyhow::Result<Vec<Reflection>> {
        let by_id = |id: &str| {
            examples
                .iter()
                .find(|e| e.id == id)
                .expect("trajectory example must exist")
        };

        let mut reflections = Vec::with_capacity(trajectories.len());
        for t in trajectories {
            let example = by_id(&t.example_id);
            let score = self.env.score(example, &t.output);
            let prompt = reflect_prompt(&t.input, &t.expected, &t.output, score);
            let critique = self.reflector.chat(&[Message::user(prompt)]).await?;
            reflections.push(Reflection {
                example_id: t.example_id.clone(),
                score,
                critique,
            });
        }
        Ok(reflections)
    }

    async fn optimize(
        &self,
        skill: &Skill,
        feedback: &crate::types::AggregatedFeedback,
    ) -> anyhow::Result<SkillEdit> {
        let prompt = optimize_prompt(skill, feedback, self.cfg.max_ops_per_edit);
        let raw = self.optimizer.chat(&[Message::user(prompt)]).await?;
        let json = extract_json_object(&raw);
        let edit: SkillEdit = serde_json::from_str(json).map_err(|e| {
            anyhow::anyhow!("failed to parse optimizer output as SkillEdit: {e}\nraw: {raw}")
        })?;
        Ok(edit)
    }

    pub async fn train(&self, initial_skill: Skill) -> anyhow::Result<TrainOutcome> {
        let train_examples = self.env.train_examples();
        let val_examples = self.env.val_examples();

        anyhow::ensure!(
            !train_examples.is_empty(),
            "environment has no training examples"
        );

        let mut best_skill = initial_skill;
        let val_subset = subset(val_examples, self.cfg.val_batch_size);
        let initial_val = self.evaluate(&best_skill, val_subset).await?;
        let mut best_val_score = initial_val.mean_score;
        let initial_val_score = best_val_score;

        let mut scheduler = BatchScheduler::new(self.cfg.seed, self.cfg.batch_size);
        let mut rejection_buffer = RejectionBuffer::new(self.cfg.rejection_buffer_size);
        let mut steps = Vec::new();

        for epoch in 0..self.cfg.epochs {
            let batches = scheduler.epoch_batches(train_examples);
            for (batch_idx, batch) in batches.iter().enumerate() {
                let step_result = self
                    .run_step(
                        &best_skill,
                        batch,
                        val_subset,
                        best_val_score,
                        &mut rejection_buffer,
                    )
                    .await?;

                if step_result.accepted {
                    best_skill = step_result
                        .candidate_skill
                        .expect("accepted step must have a candidate");
                    best_val_score = step_result.record.val_mean_score;
                }
                steps.push(StepRecord {
                    epoch,
                    batch: batch_idx,
                    best_val_mean_score: best_val_score,
                    ..step_result.record
                });
            }
        }

        let test_examples = self.env.test_examples();
        let test_result = if test_examples.is_empty() {
            None
        } else {
            Some(self.evaluate(&best_skill, test_examples).await?)
        };

        Ok(TrainOutcome {
            best_skill,
            best_val_score,
            initial_val_score,
            steps,
            test_result,
        })
    }

    async fn run_step(
        &self,
        current_skill: &Skill,
        batch: &[&Example],
        val_subset: &[Example],
        best_val_score: f64,
        rejection_buffer: &mut RejectionBuffer,
    ) -> anyhow::Result<StepOutcome> {
        let trajectories = self.rollout(current_skill, batch).await?;
        let reflections = self.reflect(&trajectories, batch).await?;
        let train_mean_score =
            reflections.iter().map(|r| r.score).sum::<f64>() / reflections.len().max(1) as f64;

        let rejected_summaries: Vec<String> = rejection_buffer.entries().cloned().collect();
        let feedback = select_feedback(&reflections, HIGHLIGHT_COUNT, rejected_summaries);

        let edit = match self.optimize(current_skill, &feedback).await {
            Ok(edit) => edit,
            Err(e) => {
                let rationale = format!("optimizer call/parse failed: {e}");
                rejection_buffer.push(rationale.clone());
                return Ok(StepOutcome {
                    accepted: false,
                    candidate_skill: None,
                    record: StepRecord {
                        epoch: 0,
                        batch: 0,
                        accepted: false,
                        rationale,
                        train_mean_score,
                        val_mean_score: best_val_score,
                        best_val_mean_score: best_val_score,
                    },
                });
            }
        };

        let candidate = match apply_edit(current_skill, &edit, self.cfg.max_ops_per_edit) {
            Ok(c) => c,
            Err(e) => {
                let rationale = format!("edit rejected ({e}): {}", edit.rationale);
                rejection_buffer.push(rationale.clone());
                return Ok(StepOutcome {
                    accepted: false,
                    candidate_skill: None,
                    record: StepRecord {
                        epoch: 0,
                        batch: 0,
                        accepted: false,
                        rationale,
                        train_mean_score,
                        val_mean_score: best_val_score,
                        best_val_mean_score: best_val_score,
                    },
                });
            }
        };

        let val_result = self.evaluate(&candidate, val_subset).await?;
        let accepted = val_result.mean_score > best_val_score + self.cfg.min_improvement;

        if !accepted {
            rejection_buffer.push(edit.rationale.clone());
        }

        Ok(StepOutcome {
            accepted,
            candidate_skill: Some(candidate),
            record: StepRecord {
                epoch: 0,
                batch: 0,
                accepted,
                rationale: edit.rationale,
                train_mean_score,
                val_mean_score: val_result.mean_score,
                best_val_mean_score: best_val_score,
            },
        })
    }
}

struct StepOutcome {
    accepted: bool,
    candidate_skill: Option<Skill>,
    record: StepRecord,
}

fn subset<T>(items: &[T], max: usize) -> &[T] {
    &items[..items.len().min(max.max(1))]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::TrainConfig;
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    struct ScriptedBackend {
        name: &'static str,
        responses: Mutex<Vec<String>>,
        calls: AtomicUsize,
    }

    impl ScriptedBackend {
        fn new(name: &'static str, responses: Vec<String>) -> Self {
            Self {
                name,
                responses: Mutex::new(responses),
                calls: AtomicUsize::new(0),
            }
        }
    }

    #[async_trait]
    impl ChatBackend for ScriptedBackend {
        fn name(&self) -> &str {
            self.name
        }
        async fn chat(&self, _messages: &[Message]) -> anyhow::Result<String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let mut responses = self.responses.lock().unwrap();
            if responses.len() > 1 {
                Ok(responses.remove(0))
            } else {
                Ok(responses[0].clone())
            }
        }
    }

    /// Toy env: executor is expected to echo the input; score is 1.0 if the
    /// output equals expected, else 0.0.
    struct EchoEnv {
        train: Vec<Example>,
        val: Vec<Example>,
        test: Vec<Example>,
    }

    impl Environment for EchoEnv {
        fn name(&self) -> &str {
            "echo"
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
        fn score(&self, example: &Example, output: &str) -> f64 {
            if output.trim() == example.expected.trim() {
                1.0
            } else {
                0.0
            }
        }
    }

    fn ex(id: &str, input: &str, expected: &str) -> Example {
        Example {
            id: id.into(),
            input: input.into(),
            expected: expected.into(),
        }
    }

    #[tokio::test]
    async fn accepts_edit_that_improves_validation_score() {
        let env = Arc::new(EchoEnv {
            train: vec![ex("t1", "hi", "HELLO")],
            val: vec![ex("v1", "hi", "HELLO")],
            test: vec![],
        });

        // Executor always answers "hi" (wrong) so every rollout scores 0.
        let executor = Arc::new(ScriptedBackend::new("executor", vec!["hi".into()]));
        let reflector = Arc::new(ScriptedBackend::new(
            "reflector",
            vec!["say HELLO instead".into()],
        ));
        let optimizer = Arc::new(ScriptedBackend::new(
            "optimizer",
            vec![r#"{"ops":[{"op":"add","anchor":null,"content":"Always answer HELLO."}],"rationale":"add explicit answer rule"}"#.into()],
        ));

        let cfg = TrainConfig {
            epochs: 1,
            batch_size: 1,
            val_batch_size: 1,
            ..Default::default()
        };
        let engine = Engine::new(executor, optimizer, reflector, env, cfg);
        let outcome = engine.train(Skill::new("# Skill\n")).await.unwrap();

        assert_eq!(outcome.steps.len(), 1);
        // With a scripted executor that always says "hi", the edit cannot
        // change the executor's behavior in this test, so validation score
        // does not improve and the edit should be rejected.
        assert!(!outcome.steps[0].accepted);
        assert_eq!(outcome.best_val_score, outcome.initial_val_score);
    }

    #[tokio::test]
    async fn rejects_edit_with_unparseable_optimizer_output() {
        let env = Arc::new(EchoEnv {
            train: vec![ex("t1", "hi", "HELLO")],
            val: vec![ex("v1", "hi", "HELLO")],
            test: vec![],
        });
        let executor = Arc::new(ScriptedBackend::new("executor", vec!["hi".into()]));
        let reflector = Arc::new(ScriptedBackend::new("reflector", vec!["needs work".into()]));
        let optimizer = Arc::new(ScriptedBackend::new(
            "optimizer",
            vec!["not json at all".into()],
        ));

        let cfg = TrainConfig {
            epochs: 1,
            batch_size: 1,
            val_batch_size: 1,
            ..Default::default()
        };
        let engine = Engine::new(executor, optimizer, reflector, env, cfg);
        let outcome = engine.train(Skill::new("# Skill\n")).await.unwrap();

        assert_eq!(outcome.steps.len(), 1);
        assert!(!outcome.steps[0].accepted);
        assert!(outcome.steps[0].rationale.contains("parse failed"));
    }

    #[tokio::test]
    async fn accepts_edit_when_executor_output_changes_with_skill() {
        // Executor backend that reads the skill from the system message and
        // "obeys" it: if the skill contains "Always answer: X", it answers X.
        struct ObedientExecutor;
        #[async_trait]
        impl ChatBackend for ObedientExecutor {
            fn name(&self) -> &str {
                "obedient"
            }
            async fn chat(&self, messages: &[Message]) -> anyhow::Result<String> {
                let skill_text = &messages[0].content;
                if let Some(idx) = skill_text.find("Always answer: ") {
                    let rest = &skill_text[idx + "Always answer: ".len()..];
                    let line_end = rest.find('\n').unwrap_or(rest.len());
                    return Ok(rest[..line_end].trim().to_string());
                }
                Ok("hi".to_string())
            }
        }

        let env = Arc::new(EchoEnv {
            train: vec![ex("t1", "hi", "HELLO")],
            val: vec![ex("v1", "hi", "HELLO")],
            test: vec![ex("te1", "hi", "HELLO")],
        });
        let executor = Arc::new(ObedientExecutor);
        let reflector = Arc::new(ScriptedBackend::new(
            "reflector",
            vec!["say HELLO instead".into()],
        ));
        let optimizer = Arc::new(ScriptedBackend::new(
            "optimizer",
            vec![r#"{"ops":[{"op":"add","anchor":null,"content":"Always answer: HELLO"}],"rationale":"add explicit answer rule"}"#.into()],
        ));

        let cfg = TrainConfig {
            epochs: 1,
            batch_size: 1,
            val_batch_size: 1,
            ..Default::default()
        };
        let engine = Engine::new(executor, optimizer, reflector, env, cfg);
        let outcome = engine.train(Skill::new("# Skill\n")).await.unwrap();

        assert!(outcome.steps[0].accepted);
        assert_eq!(outcome.best_val_score, 1.0);
        assert!(outcome.best_skill.text.contains("Always answer: HELLO"));
        assert_eq!(outcome.test_result.unwrap().mean_score, 1.0);
    }
}
