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
//! The proposer may instead be the Codex CLI, with `RSI_OUTER_PROPOSER=codex`:
//!
//! | Variable | Meaning |
//! |---|---|
//! | `RSI_OUTER_MODEL` | the Codex model (`-m`); Codex's default if unset |
//! | `RSI_OUTER_CODEX` | the native `codex` binary; default `codex` on `PATH` |
//! | `CODEX_HOME` | Codex's login and state; default `~/.codex` |
//!
//! Codex uses its own login (`codex login`); no key passes through `rsi`.

use std::path::{Path, PathBuf};
use std::time::Duration;

use rsi_core::ModelId;
use rsi_runtime::codex::{CodexConfig, PASSED_ENV};
use rsi_runtime::OpenAiModel;

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
    /// The Codex CLI.
    Codex(CodexConfig),
}

/// The proposer from `RSI_OUTER_PROPOSER` (`model`, the default, or
/// `codex`) and its variables.
///
/// # Errors
/// An unknown proposer, or a missing or invalid setting for the chosen one.
pub fn outer_from_env(wall: Duration) -> Result<Outer, String> {
    let var = |name: &str| std::env::var(name).ok();
    match var("RSI_OUTER_PROPOSER").as_deref() {
        None | Some("model") => model_from_env(Role::Outer).map(Outer::Model),
        Some("codex") => codex_config(&var, wall).map(Outer::Codex),
        Some(other) => Err(format!(
            "RSI_OUTER_PROPOSER={other}: expected `model` or `codex`"
        )),
    }
}

/// The Codex settings, reading variables through `var`.
fn codex_config(
    var: &dyn Fn(&str) -> Option<String>,
    wall: Duration,
) -> Result<CodexConfig, String> {
    let program = match var("RSI_OUTER_CODEX") {
        Some(path) => PathBuf::from(path),
        None => var("PATH")
            .and_then(|path| on_path(&path, "codex"))
            .ok_or("codex is not on PATH; set RSI_OUTER_CODEX")?,
    };
    let program = program
        .canonicalize()
        .map_err(|e| format!("resolving {}: {e}", program.display()))?;
    if is_script(&program) {
        return Err(format!(
            "{} is a script; set RSI_OUTER_CODEX to the native codex binary \
             (in an npm install: vendor/<target>/bin/codex)",
            program.display()
        ));
    }
    let home = match var("CODEX_HOME") {
        Some(path) => PathBuf::from(path),
        None => var("HOME")
            .map(|home| Path::new(&home).join(".codex"))
            .ok_or("neither CODEX_HOME nor HOME is set")?,
    };
    let home = home
        .canonicalize()
        .map_err(|e| format!("CODEX_HOME {}: {e} (run `codex login`)", home.display()))?;
    let model = var("RSI_OUTER_MODEL");
    if let Some(model) = &model {
        ModelId::parse(model).map_err(|e| e.to_string())?;
    }
    let env = PASSED_ENV
        .iter()
        .filter_map(|name| var(name).map(|v| ((*name).to_owned(), v)))
        .collect();
    Ok(CodexConfig {
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
                format!("/nonexistent:{}", dir.join("bin").display()),
            ),
            ("HOME", dir.join("home").display().to_string()),
            ("RSI_OUTER_MODEL", "gpt-x".to_owned()),
            ("HTTPS_PROXY", "http://proxy:3128".to_owned()),
            ("OPENAI_API_KEY", "never-passed".to_owned()),
        ];
        let config = codex_config(&lookup(&vars), Duration::from_secs(9)).expect("config");
        assert_eq!(config.program, dir.join("bin/codex"));
        assert_eq!(config.home, dir.join("home/.codex"));
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
                "codex login",
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
            let error = codex_config(&lookup(&vars), Duration::from_secs(1)).expect_err(expected);
            assert!(error.contains(expected), "{error}");
        }
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }
}
