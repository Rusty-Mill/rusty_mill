//! `rusty_orch lens check`: load the lens config and the operator's
//! assertions, build every provider, and report what would be admitted,
//! without calling a model or inspecting a container (#594).

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use orch_lens::config::ProviderKind;
use orch_lens::LensConfig;

use super::assertions::{self, Grants};
use super::registry::{Assertions, Clearance, Registry};
use crate::args::ArgsError;

/// Where the operator keeps the assertions file on the runner's host.
pub const DEFAULT_ASSERTIONS: &str = "/etc/let-em-loose/assertions.json";

pub const USAGE: &str = "usage: rusty_orch lens check <config.json> [--assertions <file>] [--repo <dir>]\n\nLoad the lens config and the operator's assertions (default /etc/let-em-loose/assertions.json),\nbuild every provider and report which would be admitted. Calls no model and inspects no\ncontainer: logins and the container are checked only when a question is asked.";

/// What to check.
#[derive(Debug, PartialEq, Eq)]
pub struct Options {
    pub config: PathBuf,
    pub assertions: PathBuf,
    pub repo: PathBuf,
}

/// Parse `check <config> [--assertions <file>] [--repo <dir>]`.
pub fn options(argv: impl IntoIterator<Item = String>, cwd: PathBuf) -> Result<Options, ArgsError> {
    let mut args = argv.into_iter();
    match args.next().as_deref() {
        Some("check") => {}
        Some("-h" | "--help") | None => return Err(ArgsError::Help),
        Some(other) => return Err(ArgsError::Usage(format!("unknown lens command {other:?}"))),
    }
    let (mut config, mut assertions, mut repo) = (None, None, None);
    while let Some(arg) = args.next() {
        let slot = match arg.as_str() {
            "-h" | "--help" => return Err(ArgsError::Help),
            "--assertions" => &mut assertions,
            "--repo" => &mut repo,
            _ if arg.starts_with('-') => {
                return Err(ArgsError::Usage(format!("unexpected argument {arg:?}")))
            }
            _ if config.is_none() => {
                config = Some(PathBuf::from(arg));
                continue;
            }
            _ => return Err(ArgsError::Usage(format!("unexpected argument {arg:?}"))),
        };
        if slot.is_some() {
            return Err(ArgsError::Usage(format!("duplicate {arg}")));
        }
        let value = args
            .next()
            .filter(|v| !v.is_empty() && !v.starts_with('-'))
            .ok_or_else(|| ArgsError::Usage(format!("{arg} needs a path")))?;
        *slot = Some(PathBuf::from(value));
    }
    Ok(Options {
        config: config.ok_or_else(|| ArgsError::Usage("a lens config is required".into()))?,
        assertions: assertions.unwrap_or_else(|| PathBuf::from(DEFAULT_ASSERTIONS)),
        repo: repo.unwrap_or(cwd),
    })
}

/// Check the config and assertions named in `o`. An unusable assertions
/// file is reported and treated as no assertions (fail closed); a bad
/// config or provider setting is an error.
pub fn run(o: &Options) -> Result<String, String> {
    let text = std::fs::read_to_string(&o.config)
        .map_err(|e| format!("cannot read {}: {e}", o.config.display()))?;
    let config = LensConfig::parse(&text).map_err(|e| format!("{}: {e}", o.config.display()))?;
    let (grants, note) = match assertions::load(&o.assertions) {
        Ok(g) => (g, None),
        Err(e) => (Grants::none(), Some(e)),
    };
    Registry::build(&config, &o.repo, &grants)?;
    Ok(report(&config, &grants, &o.repo, note.as_deref()))
}

/// One line per provider: its kind, what the operator asserts for it, and
/// whether it would be admitted. `note` explains ignored assertions.
pub fn report(config: &LensConfig, grants: &Grants, repo: &Path, note: Option<&str>) -> String {
    let mut out = String::new();
    if let Some(n) = note {
        let _ = writeln!(out, "assertions ignored: {n}");
    }
    // By name: `AgentId`'s own order is not alphabetical.
    let mut providers: Vec<_> = config.providers.iter().collect();
    providers.sort_by_key(|(id, _)| id.to_string());
    for (id, p) in providers {
        let a = grants.for_provider(*id);
        let (kind, verdict) = match p.kind {
            ProviderKind::ClaudeCli => ("claude-cli", claude(&a)),
            ProviderKind::CodexCli => ("codex-cli", codex(&a, repo)),
            ProviderKind::OpenAiHttp => ("openai-http", "ready".to_owned()),
        };
        let _ = writeln!(out, "{id}\t{kind}\t{verdict}");
    }
    out
}

fn claude(a: &Assertions) -> String {
    match &a.no_overage {
        Some(n) => format!("ready if logged in as {}", n.account),
        None => "unavailable: no no-overage assertion".to_owned(),
    }
}

fn codex(a: &Assertions, repo: &Path) -> String {
    if !a.no_overage.as_ref().is_some_and(|n| n.machine_bound) {
        return "unavailable: no machine-bound no-overage assertion".to_owned();
    }
    match &a.host_cleared {
        None => "unavailable: no host-cleared assertion".to_owned(),
        Some(Clearance::Host) => "ready on this host if logged in to ChatGPT".to_owned(),
        Some(Clearance::Container(spec)) => match spec.workdir_for(repo) {
            Some(dir) => format!(
                "ready in container {} at {dir} if it checks out at each call",
                spec.name
            ),
            None => format!(
                "unavailable: {} is not mounted in container {}",
                repo.display(),
                spec.name
            ),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lens::container::{ContainerSpec, Mount};
    use crate::lens::registry::NoOverage;
    use orch_core::AgentId;

    fn args(s: &str) -> Vec<String> {
        s.split_whitespace().map(str::to_owned).collect()
    }

    #[test]
    fn options_default_the_assertions_and_the_repo() {
        let o = options(args("check lens.json"), PathBuf::from("/cwd")).expect("parses");
        assert_eq!(
            o,
            Options {
                config: PathBuf::from("lens.json"),
                assertions: PathBuf::from(DEFAULT_ASSERTIONS),
                repo: PathBuf::from("/cwd"),
            }
        );
        let o = options(
            args("check --repo /r lens.json --assertions /a.json"),
            PathBuf::from("/cwd"),
        )
        .expect("parses");
        assert_eq!(o.assertions, PathBuf::from("/a.json"));
        assert_eq!(o.repo, PathBuf::from("/r"));
    }

    #[test]
    fn bad_arguments_are_usage_errors() {
        let cwd = || PathBuf::from("/cwd");
        assert_eq!(options(args(""), cwd()), Err(ArgsError::Help));
        assert_eq!(options(args("check -h"), cwd()), Err(ArgsError::Help));
        for bad in [
            "run lens.json",
            "check",
            "check a.json b.json",
            "check a.json --repo",
            "check a.json --repo /r --repo /s",
            "check a.json --assertions --repo",
            "check a.json --model x",
        ] {
            assert!(
                matches!(options(args(bad), cwd()), Err(ArgsError::Usage(_))),
                "{bad}"
            );
        }
    }

    const CONFIG: &str = r#"{
      "providers": {
        "claude": {"kind": "claude-cli", "context_tokens": 32768, "price": {"input": 0, "output": 0, "cached_input": 0}},
        "codex": {"kind": "codex-cli", "context_tokens": 32768, "price": {"input": 0, "output": 0, "cached_input": 0}},
        "local": {"kind": "openai-http", "context_tokens": 32768, "price": {"input": 0, "output": 0, "cached_input": 0},
                  "base_url": "http://127.0.0.1:8080/v1", "model": "qwen"}
      },
      "ceilings": {"experiment_usd": 0, "task_usd": 0, "task_calls": 10},
      "lenses": [
        {"id": "research", "prompt": "p", "domains": ["*"],
         "may_post": {"evidence": ["counter"]}, "tools": "read-only",
         "models": {"routine": ["claude", "codex"], "contested": ["claude"]}}
      ]
    }"#;

    fn id(s: &str) -> AgentId {
        AgentId::new(s).expect("id")
    }

    fn config() -> LensConfig {
        LensConfig::parse(CONFIG).expect("config")
    }

    #[test]
    fn nothing_asserted_leaves_only_the_http_provider_ready() {
        let r = report(
            &config(),
            &Grants::none(),
            Path::new("/repo"),
            Some("no file"),
        );
        assert_eq!(
            r,
            "assertions ignored: no file\n\
             claude\tclaude-cli\tunavailable: no no-overage assertion\n\
             codex\tcodex-cli\tunavailable: no machine-bound no-overage assertion\n\
             local\topenai-http\tready\n"
        );
    }

    #[test]
    fn asserted_providers_report_what_is_still_checked_at_call_time() {
        let spec = ContainerSpec {
            name: "lel-codex".to_owned(),
            machine_id: "lel-1".to_owned(),
            mounts: vec![Mount::Bind {
                source: PathBuf::from("/srv/lel/experiment"),
                destination: "/work".to_owned(),
                rw: true,
            }],
        };
        let no_overage = |bound| {
            Some(NoOverage {
                account: "nano@example.com".to_owned(),
                machine_bound: bound,
            })
        };
        let grants: Grants = [
            (
                id("claude"),
                Assertions {
                    no_overage: no_overage(false),
                    host_cleared: None,
                },
            ),
            (
                id("codex"),
                Assertions {
                    no_overage: no_overage(true),
                    host_cleared: Some(Clearance::Container(spec)),
                },
            ),
        ]
        .into_iter()
        .collect();
        let r = report(&config(), &grants, Path::new("/srv/lel/experiment"), None);
        assert!(
            r.contains("claude\tclaude-cli\tready if logged in as nano@example.com\n"),
            "{r}"
        );
        assert!(
            r.contains("codex\tcodex-cli\tready in container lel-codex at /work if it checks out at each call\n"),
            "{r}"
        );
        let r = report(&config(), &grants, Path::new("/home/nano/repo"), None);
        assert!(
            r.contains("codex\tcodex-cli\tunavailable: /home/nano/repo is not mounted in container lel-codex\n"),
            "{r}"
        );
    }

    #[test]
    fn a_missing_config_is_an_error() {
        let o = Options {
            config: PathBuf::from("/nonexistent/lens.json"),
            assertions: PathBuf::from("/nonexistent/assertions.json"),
            repo: PathBuf::from("/repo"),
        };
        let e = run(&o).expect_err("no config");
        assert!(e.contains("cannot read /nonexistent/lens.json"), "{e}");
    }

    #[test]
    fn a_valid_config_with_no_assertions_file_reports_without_calls() {
        let dir = std::env::temp_dir().join(format!("orch-lens-check-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("lens.json");
        std::fs::write(&path, CONFIG).expect("write");
        let o = Options {
            config: path,
            assertions: dir.join("absent.json"),
            repo: PathBuf::from("/repo"),
        };
        let r = run(&o).expect("checks");
        std::fs::remove_dir_all(&dir).expect("cleanup");
        assert!(r.contains("local\topenai-http\tready\n"), "{r}");
        assert!(!r.contains("assertions ignored"), "{r}");
    }
}
