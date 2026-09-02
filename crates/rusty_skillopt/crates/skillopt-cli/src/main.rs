use std::path::PathBuf;

use clap::{Parser, Subcommand};
use skillopt_core::{Engine, RunConfig, Skill};

#[derive(Parser)]
#[command(
    name = "skillopt",
    version,
    about = "Hand-rolled Rust reimplementation of the SkillOpt training loop"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run the full rollout -> reflect -> optimize -> validation-gate loop
    /// and write out best_skill.md plus a training report.
    Train {
        #[arg(long)]
        config: PathBuf,
    },
    /// Evaluate an existing skill document against an environment's held-out
    /// split without doing any optimization.
    Eval {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        skill: PathBuf,
        #[arg(long, default_value = "test")]
        split: Split,
    },
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum Split {
    Train,
    Val,
    Test,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();
    match cli.command {
        Command::Train { config } => run_train(&config).await,
        Command::Eval {
            config,
            skill,
            split,
        } => run_eval(&config, &skill, split).await,
    }
}

async fn run_train(config_path: &std::path::Path) -> anyhow::Result<()> {
    let cfg = RunConfig::from_file(config_path)?;
    let skill_text = std::fs::read_to_string(&cfg.skill_path)
        .map_err(|e| anyhow::anyhow!("failed to read skill_path {:?}: {e}", cfg.skill_path))?;

    let executor = skillopt_model::build_backend(&cfg.executor)?;
    let optimizer = skillopt_model::build_backend(&cfg.optimizer)?;
    let reflector = skillopt_model::build_backend(&cfg.reflector)?;
    let env = skillopt_envs::build_env(&cfg.env)?;

    println!(
        "environment {:?}: {} train / {} val / {} test examples",
        env.name(),
        env.train_examples().len(),
        env.val_examples().len(),
        env.test_examples().len()
    );

    let engine = Engine::new(executor, optimizer, reflector, env, cfg.train.clone());
    let outcome = engine.train(Skill::new(skill_text)).await?;

    std::fs::create_dir_all(&cfg.output_dir)?;
    let best_skill_path = cfg.output_dir.join("best_skill.md");
    std::fs::write(&best_skill_path, &outcome.best_skill.text)?;
    let report_path = cfg.output_dir.join("report.json");
    std::fs::write(&report_path, serde_json::to_string_pretty(&outcome)?)?;

    let accepted = outcome.steps.iter().filter(|s| s.accepted).count();
    println!(
        "training complete: {}/{} steps accepted, val score {:.3} -> {:.3}",
        accepted,
        outcome.steps.len(),
        outcome.initial_val_score,
        outcome.best_val_score
    );
    if let Some(test) = &outcome.test_result {
        println!("test score: {:.3}", test.mean_score);
    }
    println!(
        "wrote {} and {}",
        best_skill_path.display(),
        report_path.display()
    );

    Ok(())
}

async fn run_eval(
    config_path: &std::path::Path,
    skill_path: &std::path::Path,
    split: Split,
) -> anyhow::Result<()> {
    let cfg = RunConfig::from_file(config_path)?;
    let skill_text = std::fs::read_to_string(skill_path)
        .map_err(|e| anyhow::anyhow!("failed to read skill file {:?}: {e}", skill_path))?;

    let executor = skillopt_model::build_backend(&cfg.executor)?;
    let env = skillopt_envs::build_env(&cfg.env)?;

    let examples = match split {
        Split::Train => env.train_examples(),
        Split::Val => env.val_examples(),
        Split::Test => env.test_examples(),
    };
    anyhow::ensure!(!examples.is_empty(), "selected split has no examples");

    let skill = Skill::new(skill_text);
    let mut scores = Vec::with_capacity(examples.len());
    for example in examples {
        let messages = vec![
            skillopt_core::Message::system(skillopt_core::prompts::executor_system_prompt(&skill)),
            skillopt_core::Message::user(example.input.clone()),
        ];
        let output = executor.chat(&messages).await?;
        scores.push(env.score(example, &output));
    }
    let mean = scores.iter().sum::<f64>() / scores.len() as f64;
    println!(
        "{:?}: mean score {:.3} over {} examples",
        cfg.env.name,
        mean,
        scores.len()
    );

    Ok(())
}
