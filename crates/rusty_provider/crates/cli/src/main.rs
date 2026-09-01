//! `rp-cli` -- operator config/ops tooling for rusty_provider. See
//! `print_usage` below for the full command list, or run with no
//! arguments / `--help`.

use rp_cli::setup::{self, Plan, Target};
use rp_router::Config;

/// Matches `rp-server`'s own documented default (`server.host:server.port`
/// in the README's "Running" section) -- the right default for pointing a
/// CLI tool at an rp-server running on the same machine with stock config.
const DEFAULT_BASE_URL: &str = "http://localhost:8080/v1";

fn print_usage() {
    eprintln!(
        "rp-cli -- operator config/ops tooling for rusty_provider\n\
         \n\
         USAGE:\n\
         \x20 rp-cli config check [--path <config.toml>]\n\
         \x20 rp-cli providers list [--path <config.toml>]\n\
         \x20 rp-cli keys check [--path <config.toml>]\n\
         \x20 rp-cli setup list [--targets <path>]\n\
         \x20 rp-cli setup show <name> [--targets <path>] [--base-url <url>]\n\
         \x20                          [--api-key-env <VAR>] [--config-path <path>]\n\
         \x20 rp-cli setup apply <name> --yes [--targets <path>] [--base-url <url>]\n\
         \x20                            [--api-key-env <VAR>] [--config-path <path>]\n\
         \n\
         \x20 --path defaults to \"config.toml\" in the current directory.\n\
         \x20 --base-url defaults to \"{DEFAULT_BASE_URL}\".\n\
         \n\
         config/providers/keys are read-only: no network call, no\n\
         resolved secret's value printed, only whether its env var is\n\
         set.\n\
         \n\
         setup rewrites a known third-party CLI tool's own config file to\n\
         point its endpoint at rusty_provider (see ADR-0004) -- \"show\" is\n\
         a dry run, \"apply\" requires --yes and always backs up the\n\
         previous file first. Never writes a literal API key, only an\n\
         env-var reference naming --api-key-env when the target format\n\
         supports one."
    );
}

/// Pulls `--path <value>` (or `-p <value>`) out of `args`, defaulting to
/// `"config.toml"` when absent -- the same default `cargo run -p
/// rp-server` documents in the README's "Running" section.
fn path_arg(args: &[String]) -> String {
    args.iter()
        .position(|a| a == "--path" || a == "-p")
        .and_then(|i| args.get(i + 1))
        .cloned()
        .unwrap_or_else(|| "config.toml".to_string())
}

fn load_config(path: &str) -> Option<Config> {
    match Config::from_file(path) {
        Ok(config) => Some(config),
        Err(e) => {
            eprintln!("error: {e}");
            None
        }
    }
}

fn env_lookup(var: &str) -> Option<String> {
    std::env::var(var).ok()
}

/// Pulls `<flag> <value>` out of `args`, e.g. `opt_arg(args, "--base-url")`.
fn opt_arg(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn has_flag(args: &[String], flag: &str) -> bool {
    args.iter().any(|a| a == flag)
}

/// Loads the target list for `setup {list,show,apply}`: an operator's own
/// `--targets <path>` file if given, else the list baked into this binary.
fn load_targets(args: &[String]) -> Option<Vec<Target>> {
    match opt_arg(args, "--targets") {
        Some(path) => {
            let content = match std::fs::read_to_string(&path) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("error: reading {path}: {e}");
                    return None;
                }
            };
            match setup::parse_targets(&content) {
                Ok(t) => Some(t),
                Err(e) => {
                    eprintln!("error: parsing {path}: {e}");
                    None
                }
            }
        }
        None => match setup::default_targets() {
            Ok(t) => Some(t),
            Err(e) => {
                eprintln!("error: parsing built-in cli_targets.toml: {e}");
                None
            }
        },
    }
}

fn cmd_setup_list(args: &[String]) -> i32 {
    let Some(targets) = load_targets(args) else {
        return 1;
    };
    if targets.is_empty() {
        println!("no targets configured");
        return 0;
    }
    for t in &targets {
        let expanded = setup::expand_home(&t.config_path);
        let exists = std::path::Path::new(&expanded).is_file();
        println!(
            "{:<12} {:<58} {}",
            t.name,
            t.description,
            if exists {
                format!("config file exists ({expanded})")
            } else {
                format!("config file not found, would be created ({expanded})")
            }
        );
    }
    0
}

/// Shared by `setup show` and `setup apply`: resolves the named target and
/// builds the merge plan, printing a usage/error message and returning
/// `None` on any failure.
fn resolve_plan(name: &str, args: &[String]) -> Option<Plan> {
    let targets = load_targets(args)?;
    let Some(target) = targets.iter().find(|t| t.name == name) else {
        eprintln!("error: unknown target \"{name}\" -- see `rp-cli setup list`");
        return None;
    };
    let base_url = opt_arg(args, "--base-url").unwrap_or_else(|| DEFAULT_BASE_URL.to_string());
    let api_key_env = opt_arg(args, "--api-key-env");
    let config_path_override = opt_arg(args, "--config-path");
    match setup::build_plan(
        target,
        &base_url,
        api_key_env.as_deref(),
        config_path_override.as_deref(),
    ) {
        Ok(plan) => Some(plan),
        Err(e) => {
            eprintln!("error: {e}");
            None
        }
    }
}

fn print_plan(plan: &Plan) {
    println!("target: {}", plan.target_name);
    println!("file:   {}", plan.config_path);
    println!(
        "status: {}",
        if plan.existed_before {
            "exists -- will be merged into, other keys are kept"
        } else {
            "does not exist -- will be created"
        }
    );
    if !plan.skipped_fields.is_empty() {
        println!(
            "skipped (pass --api-key-env <VAR> to include): {}",
            plan.skipped_fields.join(", ")
        );
    }
    if plan.changed {
        println!("\n--- {} (after) ---\n{}", plan.config_path, plan.after);
    } else {
        println!("\nalready up to date -- nothing to change.");
    }
}

fn cmd_setup_show(args: &[String]) -> i32 {
    let Some(name) = args.first() else {
        eprintln!(
            "usage: rp-cli setup show <name> [--targets <path>] [--base-url <url>] \
             [--api-key-env <VAR>] [--config-path <path>]"
        );
        return 1;
    };
    let Some(plan) = resolve_plan(name, &args[1..]) else {
        return 1;
    };
    print_plan(&plan);
    0
}

fn cmd_setup_apply(args: &[String]) -> i32 {
    let Some(name) = args.first() else {
        eprintln!(
            "usage: rp-cli setup apply <name> --yes [--targets <path>] [--base-url <url>] \
             [--api-key-env <VAR>] [--config-path <path>]"
        );
        return 1;
    };
    let rest = &args[1..];
    if !has_flag(rest, "--yes") {
        eprintln!(
            "error: setup apply writes to {name}'s own config file -- run \
             `rp-cli setup show {name}` first, then rerun with --yes"
        );
        return 1;
    }
    let Some(plan) = resolve_plan(name, rest) else {
        return 1;
    };
    match setup::apply_plan(&plan) {
        Ok(backup) => {
            if !plan.changed {
                println!(
                    "{} already up to date -- nothing written.",
                    plan.config_path
                );
            } else {
                if let Some(b) = &backup {
                    println!("backed up previous file to {b}");
                }
                println!("wrote {}", plan.config_path);
            }
            if !plan.skipped_fields.is_empty() {
                println!(
                    "skipped (pass --api-key-env <VAR> to include): {}",
                    plan.skipped_fields.join(", ")
                );
            }
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

fn cmd_config_check(args: &[String]) -> i32 {
    let path = path_arg(args);
    let Some(config) = load_config(&path) else {
        return 1;
    };

    println!("{path}: valid TOML, matches the config schema.\n");

    let providers = rp_cli::provider_reports(&config, env_lookup);
    let active = providers.iter().filter(|p| p.active).count();
    println!(
        "providers: {} configured, {} active, {} skipped (unresolved api_key_env)",
        providers.len(),
        active,
        providers.len() - active
    );
    for p in providers.iter().filter(|p| !p.active) {
        println!("  - {} skipped: {} is not set", p.name, p.api_key_env);
    }

    println!("route aliases: {}", config.routes.len());
    println!("clients: {}", config.clients.len());

    let guardrail_checks = rp_cli::check_guardrails(&config);
    let broken: Vec<_> = guardrail_checks
        .iter()
        .filter_map(|c| c.error.as_ref().map(|e| (&c.name, e)))
        .collect();
    if guardrail_checks.is_empty() {
        println!("guardrails: none configured");
    } else {
        println!(
            "guardrails: {} configured, {} invalid",
            guardrail_checks.len(),
            broken.len()
        );
        for (name, err) in &broken {
            println!("  - {name}: invalid pattern: {err}");
        }
    }

    println!(
        "persistence: {}",
        if config.persistence.is_some() {
            "configured"
        } else {
            "in-memory only (not configured)"
        }
    );
    println!(
        "admin API: {}",
        if config.server.admin_key_env.is_some() {
            "enabled"
        } else {
            "disabled (server.admin_key_env not set)"
        }
    );
    println!(
        "jwt auth: {}",
        match &config.jwt {
            Some(jwt) if jwt.hs256_secret_env.is_some() => "configured (hs256)".to_string(),
            Some(jwt) if jwt.jwks_url.is_some() => "configured (jwks)".to_string(),
            Some(_) => {
                "configured but neither hs256_secret_env nor jwks_url is set -- disabled at startup with a warning".to_string()
            }
            None => "not configured".to_string(),
        }
    );

    if !broken.is_empty() {
        eprintln!(
            "\nnote: an invalid guardrail pattern doesn't fail startup -- rusty_provider skips \
             it with a warning and keeps running, same as it does for the other soft-failure \
             cases above. Fix it here so the router isn't silently missing a guardrail you \
             think is active."
        );
    }

    0
}

fn cmd_providers_list(args: &[String]) -> i32 {
    let path = path_arg(args);
    let Some(config) = load_config(&path) else {
        return 1;
    };

    let providers = rp_cli::provider_reports(&config, env_lookup);
    if providers.is_empty() {
        println!("no [providers.*] entries in {path}");
        return 0;
    }

    for p in &providers {
        let status = if p.active {
            "active".to_string()
        } else {
            format!("skipped ({} not set)", p.api_key_env)
        };
        println!(
            "{:<20} {:<10} {:<42} {}",
            p.name,
            format!("{:?}", p.kind),
            p.base_url,
            status
        );
    }
    0
}

fn cmd_keys_check(args: &[String]) -> i32 {
    let path = path_arg(args);
    let Some(config) = load_config(&path) else {
        return 1;
    };

    let rows = rp_cli::key_env_reports(&config, env_lookup);
    if rows.is_empty() {
        println!("no *_env fields configured in {path}");
        return 0;
    }

    let mut unset_count = 0;
    for row in &rows {
        let mark = if row.set {
            "set"
        } else {
            unset_count += 1;
            "NOT SET"
        };
        println!("{:<45} {:<20} {}", row.label, row.env_var, mark);
    }
    if unset_count > 0 {
        println!(
            "\n{unset_count} of {} env vars not set -- the corresponding provider/client/section \
             will be skipped at startup, not treated as a hard error.",
            rows.len()
        );
    }
    0
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let exit_code = match (
        args.first().map(String::as_str),
        args.get(1).map(String::as_str),
    ) {
        (Some("config"), Some("check")) => cmd_config_check(&args[2..]),
        (Some("providers"), Some("list")) => cmd_providers_list(&args[2..]),
        (Some("keys"), Some("check")) => cmd_keys_check(&args[2..]),
        (Some("setup"), Some("list")) => cmd_setup_list(&args[2..]),
        (Some("setup"), Some("show")) => cmd_setup_show(&args[2..]),
        (Some("setup"), Some("apply")) => cmd_setup_apply(&args[2..]),
        (Some("--help") | Some("-h"), _) | (None, _) => {
            print_usage();
            0
        }
        _ => {
            print_usage();
            1
        }
    };
    std::process::exit(exit_code);
}
