//! Model configuration from the environment, one set of variables per role.
//!
//! | Variable | Meaning |
//! |---|---|
//! | `RSI_<ROLE>_MODEL` | the model id (required) |
//! | `RSI_<ROLE>_BASE_URL` | an OpenAI-compatible base URL; default a local Ollama |
//! | `RSI_<ROLE>_API_KEY` | a bearer token, if the endpoint needs one |
//!
//! `<ROLE>` is `INNER` (the agent under test) or `OUTER` (the proposer).
//! Secrets come only from the environment and are never written anywhere.
//!
//! A coding-agent CLI may serve instead, each with its own login (`codex
//! login`, or `/login` in `claude`), so no key passes through `rsi`:
//!
//! | Variable | Meaning |
//! |---|---|
//! | `RSI_OUTER_PROPOSER` | `model` (default), `codex` or `claude` |
//! | `RSI_INNER_PROVIDER` | `openai` (default) or `codex` |
//! | `RSI_<ROLE>_CODEX`, `RSI_OUTER_CLAUDE` | the native binary; default `codex` / `claude` on `PATH` |
//! | `RSI_<ROLE>_MODEL` | the agent's model; its default if unset |
//! | `CODEX_HOME`, `CLAUDE_CONFIG_DIR` | the login and state; default `~/.codex`, `~/.claude` |

use std::path::{Path, PathBuf};
use std::time::Duration;

use rsi_core::{ChatModel, Completion, Message, ModelId};
use rsi_runtime::agent_cli::{CliAgent, CliConfig, PASSED_ENV};
use rsi_runtime::codex_model::CodexModel;
use rsi_runtime::{OpenAiModel, ProcessExecutor, RuntimeError};

/// The default endpoint: a local Ollama.
pub const DEFAULT_BASE_URL: &str = "http://127.0.0.1:11434/v1";

/// The longest a single model call may take (budgeted calls end sooner).
const MODEL_TIMEOUT: Duration = Duration::from_secs(900);

/// Which model a command needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// The inner agent's model.
    Inner,
    /// The outer proposer's model.
    Outer,
}

impl Role {
    const fn prefix(self) -> &'static str {
        match self {
            Self::Inner => "RSI_INNER",
            Self::Outer => "RSI_OUTER",
        }
    }
}

/// The model for `role`, from `RSI_<ROLE>_*`.
///
/// # Errors
/// When `RSI_<ROLE>_MODEL` is unset, or the id or URL is invalid.
pub fn model_from_env(role: Role) -> Result<OpenAiModel, String> {
    let var = |name: &str| std::env::var(format!("{}_{name}", role.prefix())).ok();
    let id = var("MODEL").ok_or_else(|| format!("{}_MODEL is not set", role.prefix()))?;
    let base_url = var("BASE_URL").unwrap_or_else(|| DEFAULT_BASE_URL.to_owned());
    OpenAiModel::new(
        ModelId::parse(&id).map_err(|e| e.to_string())?,
        &base_url,
        var("API_KEY"),
        MODEL_TIMEOUT,
    )
    .map_err(|e| e.to_string())
}

/// The outer loop's proposer, as configured.
#[derive(Debug)]
pub enum Outer {
    /// A chat model over an OpenAI-compatible endpoint.
    Model(OpenAiModel),
    /// A coding-agent CLI (Codex or Claude Code).
    Cli(CliConfig),
}

/// The proposer from `RSI_OUTER_PROPOSER` (`model`, the default, `codex`
/// or `claude`) and its variables.
///
/// # Errors
/// An unknown proposer, or a missing or invalid setting for the chosen one.
pub fn outer_from_env(wall: Duration) -> Result<Outer, String> {
    let var = |name: &str| std::env::var(name).ok();
    let agent = match var("RSI_OUTER_PROPOSER").as_deref() {
        None | Some("model") => return model_from_env(Role::Outer).map(Outer::Model),
        Some("codex") => CliAgent::Codex,
        Some("claude") => CliAgent::Claude,
        Some(other) => {
            return Err(format!(
                "RSI_OUTER_PROPOSER={other}: expected `model`, `codex` or `claude`"
            ))
        }
    };
    cli_config(&var, agent, Role::Outer, wall).map(Outer::Cli)
}

/// The inner agent's model: an OpenAI-compatible endpoint or Codex.
#[derive(Debug)]
pub enum Inner {
    /// An OpenAI-compatible endpoint.
    OpenAi(OpenAiModel),
    /// The Codex CLI, in the sandbox.
    Codex(CodexModel),
}

impl ChatModel for Inner {
    type Error = RuntimeError;

    fn id(&self) -> &ModelId {
        match self {
            Self::OpenAi(model) => model.id(),
            Self::Codex(model) => model.id(),
        }
    }

    fn complete(
        &self,
        messages: &[Message],
        max_tokens: u64,
        timeout: Duration,
    ) -> Result<Completion, RuntimeError> {
        match self {
            Self::OpenAi(model) => model.complete(messages, max_tokens, timeout),
            Self::Codex(model) => model.complete(messages, max_tokens, timeout),
        }
    }
}

/// The inner model from `RSI_INNER_PROVIDER` (`openai`, the default, or
/// `codex`). Codex runs through `executor`, staging each call under
/// `scratch`, and refuses to start if its sandbox could reach a
/// `protected` path.
///
/// # Errors
/// An unknown provider, or a missing or invalid setting for the chosen one.
pub fn inner_from_env(
    executor: &ProcessExecutor,
    scratch: PathBuf,
    protected: Vec<PathBuf>,
) -> Result<Inner, String> {
    let var = |name: &str| std::env::var(name).ok();
    match var("RSI_INNER_PROVIDER").as_deref() {
        None | Some("openai") => model_from_env(Role::Inner).map(Inner::OpenAi),
        Some("codex") => {
            let config = cli_config(&var, CliAgent::Codex, Role::Inner, MODEL_TIMEOUT)?;
            CodexModel::new(executor, config, scratch, protected)
                .map(Inner::Codex)
                .map_err(|e| e.to_string())
        }
        Some(other) => Err(format!(
            "RSI_INNER_PROVIDER={other}: expected `openai` or `codex`"
        )),
    }
}

/// A coding-agent CLI's settings for `role`, reading variables through
/// `var`.
fn cli_config(
    var: &dyn Fn(&str) -> Option<String>,
    agent: CliAgent,
    role: Role,
    wall: Duration,
) -> Result<CliConfig, String> {
    let name = agent.name();
    let program_var = format!("{}_{}", role.prefix(), name.to_uppercase());
    let program = match var(&program_var) {
        Some(path) => PathBuf::from(path),
        None => var("PATH")
            .and_then(|path| on_path(&path, name))
            .ok_or_else(|| format!("{name} is not on PATH; set {program_var}"))?,
    };
    let program = program
        .canonicalize()
        .map_err(|e| format!("resolving {}: {e}", program.display()))?;
    if is_script(&program) {
        return Err(format!(
            "{} is a script; set {program_var} to the native {name} binary",
            program.display()
        ));
    }
    let home_var = agent.home_var();
    let home = match var(home_var) {
        Some(path) => PathBuf::from(path),
        None => var("HOME")
            .map(|home| Path::new(&home).join(format!(".{name}")))
            .ok_or_else(|| format!("neither {home_var} nor HOME is set"))?,
    };
    let home = home.canonicalize().map_err(|e| {
        format!(
            "{home_var} {}: {e} (log in to {name} first)",
            home.display()
        )
    })?;
    let model = var(&format!("{}_MODEL", role.prefix()));
    if let Some(model) = &model {
        ModelId::parse(model).map_err(|e| e.to_string())?;
    }
    let env = PASSED_ENV
        .iter()
        .filter_map(|name| var(name).map(|v| ((*name).to_owned(), v)))
        .collect();
    Ok(CliConfig {
        agent,
        program,
        model,
        home,
        wall,
        env,
    })
}

/// Whether `path` starts with `#!`, as the npm package's launcher does.
fn is_script(path: &Path) -> bool {
    let mut head = [0_u8; 2];
    std::fs::File::open(path)
        .and_then(|mut file| std::io::Read::read_exact(&mut file, &mut head))
        .is_ok_and(|()| head == *b"#!")
}

fn on_path(path: &str, name: &str) -> Option<PathBuf> {
    std::env::split_paths(path)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rsi-config-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("bin")).expect("bin");
        std::fs::create_dir_all(dir.join("home/.codex")).expect("home");
        std::fs::create_dir_all(dir.join("home/.claude")).expect("claude home");
        dir.canonicalize().expect("canonical")
    }

    fn lookup<'a>(vars: &'a [(&str, String)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |name| {
            vars.iter()
                .find(|(n, _)| *n == name)
                .map(|(_, v)| v.clone())
        }
    }

    #[test]
    fn codex_is_found_on_path_with_its_home_model_and_proxy() {
        let dir = scratch("found");
        std::fs::write(dir.join("bin/codex"), b"\x7fELF").expect("binary");
        let vars = [
            (
                "PATH",
                std::env::join_paths([PathBuf::from("/nonexistent"), dir.join("bin")])
                    .expect("joinable")
                    .into_string()
                    .expect("utf-8"),
            ),
            ("HOME", dir.join("home").display().to_string()),
            ("RSI_OUTER_MODEL", "gpt-x".to_owned()),
            ("HTTPS_PROXY", "http://proxy:3128".to_owned()),
            ("OPENAI_API_KEY", "never-passed".to_owned()),
        ];
        let config = cli_config(
            &lookup(&vars),
            CliAgent::Codex,
            Role::Outer,
            Duration::from_secs(9),
        )
        .expect("config");
        let canonical = |path: PathBuf| path.canonicalize().expect("exists");
        assert_eq!(config.program, canonical(dir.join("bin").join("codex")));
        assert_eq!(config.home, canonical(dir.join("home").join(".codex")));
        assert_eq!(config.model.as_deref(), Some("gpt-x"));
        assert_eq!(
            config.env,
            vec![("HTTPS_PROXY".to_owned(), "http://proxy:3128".to_owned())],
            "only proxy and certificate settings pass"
        );
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }

    #[test]
    fn a_script_launcher_a_missing_home_or_a_bad_model_is_refused() {
        let dir = scratch("refused");
        std::fs::write(dir.join("bin/codex"), "#!/usr/bin/env node\n").expect("script");
        std::fs::write(dir.join("bin/native"), b"\x7fELF").expect("binary");
        let codex = |file: &str| ("RSI_OUTER_CODEX", dir.join(file).display().to_string());
        let home = ("CODEX_HOME", dir.join("home/.codex").display().to_string());
        let cases = [
            (vec![codex("bin/codex"), home.clone()], "is a script"),
            (
                vec![
                    codex("bin/native"),
                    ("CODEX_HOME", "/nonexistent".to_owned()),
                ],
                "log in to codex",
            ),
            (
                vec![
                    codex("bin/native"),
                    home,
                    ("RSI_OUTER_MODEL", "a b".to_owned()),
                ],
                "model id",
            ),
            (vec![("PATH", "/nonexistent".to_owned())], "not on PATH"),
        ];
        for (vars, expected) in cases {
            let error = cli_config(
                &lookup(&vars),
                CliAgent::Codex,
                Role::Outer,
                Duration::from_secs(1),
            )
            .expect_err(expected);
            assert!(error.contains(expected), "{error}");
        }
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }

    #[test]
    fn each_agent_and_role_reads_its_own_variables() {
        let dir = scratch("roles");
        std::fs::write(dir.join("bin/claude"), b"\x7fELF").expect("claude");
        std::fs::write(dir.join("bin/codex"), b"\x7fELF").expect("codex");
        let vars = [
            (
                "RSI_OUTER_CLAUDE",
                dir.join("bin/claude").display().to_string(),
            ),
            (
                "RSI_INNER_CODEX",
                dir.join("bin/codex").display().to_string(),
            ),
            ("HOME", dir.join("home").display().to_string()),
            ("RSI_OUTER_MODEL", "opus".to_owned()),
            ("RSI_INNER_MODEL", "gpt-mini".to_owned()),
        ];
        let canonical = |path: PathBuf| path.canonicalize().expect("exists");
        let claude = cli_config(
            &lookup(&vars),
            CliAgent::Claude,
            Role::Outer,
            Duration::from_secs(1),
        )
        .expect("claude");
        assert_eq!(claude.home, canonical(dir.join("home").join(".claude")));
        assert_eq!(claude.model.as_deref(), Some("opus"));
        let inner = cli_config(
            &lookup(&vars),
            CliAgent::Codex,
            Role::Inner,
            Duration::from_secs(1),
        )
        .expect("inner codex");
        assert_eq!(inner.program, canonical(dir.join("bin").join("codex")));
        assert_eq!(inner.model.as_deref(), Some("gpt-mini"));
        let e = cli_config(
            &lookup(&vars[2..]),
            CliAgent::Claude,
            Role::Outer,
            Duration::from_secs(1),
        )
        .expect_err("no binary");
        assert!(e.contains("RSI_OUTER_CLAUDE"), "{e}");
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }
}
