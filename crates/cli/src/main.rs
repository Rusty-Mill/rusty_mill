//! `rp-cli` -- operator config/ops tooling for rusty_provider. See
//! `print_usage` below for the full command list, or run with no
//! arguments / `--help`.

use rp_router::Config;

fn print_usage() {
    eprintln!(
        "rp-cli -- operator config/ops tooling for rusty_provider\n\
         \n\
         USAGE:\n\
         \x20 rp-cli config check [--path <config.toml>]\n\
         \x20 rp-cli providers list [--path <config.toml>]\n\
         \x20 rp-cli keys check [--path <config.toml>]\n\
         \n\
         \x20 --path defaults to \"config.toml\" in the current directory.\n\
         \n\
         Read-only: this never makes a network call and never prints a\n\
         resolved secret's value, only whether its env var is set."
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
