//! `rp-cli setup` -- static config-file rewriting for third-party CLI
//! tools that already read their provider/endpoint settings from a local
//! JSON or TOML file. See `docs/adr/0004-cli-target-config-rewriting.md`
//! for why this is scoped to file rewriting (no proxy, no traffic
//! interception) and why the tool list is data (`cli_targets.toml`), not
//! Rust code.
//!
//! Same rule `rp-cli`'s other commands follow for secrets -- never handle
//! a resolved value -- extends here to writing: a field that needs an API
//! key gets the target tool's own env-var-reference syntax naming
//! whatever variable the caller passed, never a literal token, and this
//! module never reads that variable itself.

use serde::Deserialize;

const DEFAULT_TARGETS_TOML: &str = include_str!("../cli_targets.toml");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Json,
    Toml,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FieldSpec {
    pub path: String,
    pub value: String,
    #[serde(default)]
    pub requires_api_key_env: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Target {
    pub name: String,
    pub description: String,
    pub config_path: String,
    pub format: Format,
    #[serde(default)]
    pub fields: Vec<FieldSpec>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct TargetsFile {
    #[serde(default)]
    targets: Vec<Target>,
}

/// Parses a `cli_targets.toml`-shaped document (whether the built-in
/// default or an operator-supplied `--targets <path>` file) into its list
/// of targets.
pub fn parse_targets(toml_str: &str) -> Result<Vec<Target>, String> {
    toml::from_str::<TargetsFile>(toml_str)
        .map(|f| f.targets)
        .map_err(|e| e.to_string())
}

/// The target list baked into the `rp-cli` binary at compile time.
pub fn default_targets() -> Result<Vec<Target>, String> {
    parse_targets(DEFAULT_TARGETS_TOML)
}

/// Expands a leading `~` (or `~/...`) using `$HOME`. Any other path is
/// returned unchanged -- this is deliberately not a general tilde/glob
/// expander, just enough to make `cli_targets.toml`'s `~/.config/...`
/// paths usable.
pub fn expand_home(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return format!("{}/{rest}", home.trim_end_matches('/'));
        }
    } else if path == "~" {
        if let Ok(home) = std::env::var("HOME") {
            return home;
        }
    }
    path.to_string()
}

fn render_value(template: &str, base_url: &str, api_key_env: Option<&str>) -> String {
    let mut rendered = template.replace("{{base_url}}", base_url);
    if let Some(env) = api_key_env {
        rendered = rendered.replace("{{api_key_env}}", env);
    }
    rendered
}

fn path_prefix_label(path: &[&str], i: usize) -> String {
    if i == 0 {
        "the file root".to_string()
    } else {
        format!("\"{}\"", path[..i].join("."))
    }
}

fn set_json_path(
    root: &mut serde_json::Value,
    path: &[&str],
    value: serde_json::Value,
) -> Result<(), String> {
    let mut cur = root;
    for (i, seg) in path.iter().enumerate() {
        let last = i + 1 == path.len();
        let map = cur.as_object_mut().ok_or_else(|| {
            format!(
                "cannot set \"{}\": {} is not an object in the existing file",
                path.join("."),
                path_prefix_label(path, i)
            )
        })?;
        if last {
            map.insert((*seg).to_string(), value);
            return Ok(());
        }
        cur = map
            .entry((*seg).to_string())
            .or_insert_with(|| serde_json::Value::Object(Default::default()));
    }
    Ok(())
}

fn set_toml_path(root: &mut toml::Value, path: &[&str], value: toml::Value) -> Result<(), String> {
    let mut cur = root;
    for (i, seg) in path.iter().enumerate() {
        let last = i + 1 == path.len();
        let table = cur.as_table_mut().ok_or_else(|| {
            format!(
                "cannot set \"{}\": {} is not a table in the existing file",
                path.join("."),
                path_prefix_label(path, i)
            )
        })?;
        if last {
            table.insert((*seg).to_string(), value);
            return Ok(());
        }
        cur = table
            .entry((*seg).to_string())
            .or_insert_with(|| toml::Value::Table(Default::default()));
    }
    Ok(())
}

/// What `setup show`/`setup apply` would do to one target's config file --
/// computed without touching disk (`build_plan` only reads the existing
/// file, if any; only `apply_plan` writes).
#[derive(Debug, Clone)]
pub struct Plan {
    pub target_name: String,
    pub config_path: String,
    pub existed_before: bool,
    pub before: String,
    pub after: String,
    pub changed: bool,
    /// Field paths skipped because they need `--api-key-env` and it
    /// wasn't given -- never because of an error.
    pub skipped_fields: Vec<String>,
}

/// Builds the merge plan for `target`: read whatever's already at its
/// config path (or start empty), set every applicable field, and render
/// the result -- without writing anything.
pub fn build_plan(
    target: &Target,
    base_url: &str,
    api_key_env: Option<&str>,
    config_path_override: Option<&str>,
) -> Result<Plan, String> {
    let raw_path = config_path_override.unwrap_or(&target.config_path);
    let config_path = expand_home(raw_path);

    let existed_before = std::path::Path::new(&config_path).is_file();
    let before = if existed_before {
        std::fs::read_to_string(&config_path).map_err(|e| format!("reading {config_path}: {e}"))?
    } else {
        String::new()
    };

    let mut skipped_fields = Vec::new();
    let mut applicable = Vec::new();
    for field in &target.fields {
        if field.requires_api_key_env && api_key_env.is_none() {
            skipped_fields.push(field.path.clone());
            continue;
        }
        applicable.push(field);
    }

    let after = match target.format {
        Format::Json => {
            let mut root: serde_json::Value = if existed_before {
                serde_json::from_str(&before)
                    .map_err(|e| format!("parsing {config_path} as JSON: {e}"))?
            } else {
                serde_json::Value::Object(Default::default())
            };
            for field in &applicable {
                let value = render_value(&field.value, base_url, api_key_env);
                let path_parts: Vec<&str> = field.path.split('.').collect();
                set_json_path(&mut root, &path_parts, serde_json::Value::String(value))?;
            }
            let mut rendered = serde_json::to_string_pretty(&root)
                .map_err(|e| format!("serializing {config_path}: {e}"))?;
            rendered.push('\n');
            rendered
        }
        Format::Toml => {
            let mut root: toml::Value = if existed_before {
                before
                    .parse::<toml::Value>()
                    .map_err(|e| format!("parsing {config_path} as TOML: {e}"))?
            } else {
                toml::Value::Table(Default::default())
            };
            for field in &applicable {
                let value = render_value(&field.value, base_url, api_key_env);
                let path_parts: Vec<&str> = field.path.split('.').collect();
                set_toml_path(&mut root, &path_parts, toml::Value::String(value))?;
            }
            toml::to_string_pretty(&root).map_err(|e| format!("serializing {config_path}: {e}"))?
        }
    };

    let changed = after != before;
    Ok(Plan {
        target_name: target.name.clone(),
        config_path,
        existed_before,
        before,
        after,
        changed,
        skipped_fields,
    })
}

/// Writes `plan.after` to `plan.config_path`, backing up whatever was
/// there first (`<path>.bak`). No-op if the plan reported no change.
/// Returns the backup path written, if any.
pub fn apply_plan(plan: &Plan) -> Result<Option<String>, String> {
    if !plan.changed {
        return Ok(None);
    }
    if let Some(parent) = std::path::Path::new(&plan.config_path).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("creating {}: {e}", parent.display()))?;
        }
    }
    let backup_path = if plan.existed_before {
        let backup = format!("{}.bak", plan.config_path);
        std::fs::copy(&plan.config_path, &backup)
            .map_err(|e| format!("backing up {} to {backup}: {e}", plan.config_path))?;
        Some(backup)
    } else {
        None
    };
    std::fs::write(&plan.config_path, &plan.after)
        .map_err(|e| format!("writing {}: {e}", plan.config_path))?;
    Ok(backup_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_targets_parse_and_are_non_empty() {
        let targets = default_targets().expect("built-in cli_targets.toml must parse");
        assert!(!targets.is_empty());
        for t in &targets {
            assert!(!t.fields.is_empty(), "target {} has no fields", t.name);
        }
    }

    #[test]
    fn expand_home_replaces_leading_tilde() {
        std::env::set_var("HOME", "/home/op");
        assert_eq!(
            expand_home("~/.config/foo.json"),
            "/home/op/.config/foo.json"
        );
        assert_eq!(expand_home("/already/absolute"), "/already/absolute");
    }

    fn json_target() -> Target {
        Target {
            name: "test-json".to_string(),
            description: "test".to_string(),
            config_path: "unused".to_string(),
            format: Format::Json,
            fields: vec![
                FieldSpec {
                    path: "provider.rp.options.baseURL".to_string(),
                    value: "{{base_url}}".to_string(),
                    requires_api_key_env: false,
                },
                FieldSpec {
                    path: "provider.rp.options.apiKey".to_string(),
                    value: "{env:{{api_key_env}}}".to_string(),
                    requires_api_key_env: true,
                },
            ],
        }
    }

    #[test]
    fn build_plan_creates_a_new_json_file_from_scratch() {
        let target = json_target();
        let plan = build_plan(
            &target,
            "http://localhost:8080/v1",
            None,
            Some("/no/such/file"),
        )
        .unwrap();
        assert!(!plan.existed_before);
        assert!(plan.changed);
        assert_eq!(plan.skipped_fields, vec!["provider.rp.options.apiKey"]);
        let parsed: serde_json::Value = serde_json::from_str(&plan.after).unwrap();
        assert_eq!(
            parsed["provider"]["rp"]["options"]["baseURL"],
            "http://localhost:8080/v1"
        );
        assert!(parsed["provider"]["rp"]["options"].get("apiKey").is_none());
    }

    #[test]
    fn build_plan_includes_api_key_env_reference_when_given() {
        let target = json_target();
        let plan = build_plan(
            &target,
            "http://localhost:8080/v1",
            Some("RP_KEY"),
            Some("/no/such/file"),
        )
        .unwrap();
        assert!(plan.skipped_fields.is_empty());
        let parsed: serde_json::Value = serde_json::from_str(&plan.after).unwrap();
        assert_eq!(
            parsed["provider"]["rp"]["options"]["apiKey"],
            "{env:RP_KEY}"
        );
    }

    #[test]
    fn build_plan_merges_into_an_existing_file_without_disturbing_other_keys() {
        let dir = std::env::temp_dir().join(format!("rp-cli-setup-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("opencode.json");
        std::fs::write(&path, r#"{"unrelated": {"kept": true}}"#).unwrap();

        let target = json_target();
        let plan = build_plan(
            &target,
            "http://localhost:9000/v1",
            None,
            Some(path.to_str().unwrap()),
        )
        .unwrap();
        assert!(plan.existed_before);
        let parsed: serde_json::Value = serde_json::from_str(&plan.after).unwrap();
        assert_eq!(parsed["unrelated"]["kept"], true);
        assert_eq!(
            parsed["provider"]["rp"]["options"]["baseURL"],
            "http://localhost:9000/v1"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn build_plan_refuses_to_clobber_a_non_object_intermediate() {
        let dir =
            std::env::temp_dir().join(format!("rp-cli-setup-test-conflict-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("conflict.json");
        std::fs::write(&path, r#"{"provider": "not-an-object"}"#).unwrap();

        let target = json_target();
        let err = build_plan(
            &target,
            "http://localhost:8080/v1",
            None,
            Some(path.to_str().unwrap()),
        )
        .unwrap_err();
        assert!(err.contains("provider"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn build_plan_reports_unchanged_when_rerun_with_identical_inputs() {
        let dir = std::env::temp_dir().join(format!(
            "rp-cli-setup-test-idempotent-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("opencode.json");

        let target = json_target();
        let first = build_plan(
            &target,
            "http://localhost:8080/v1",
            None,
            Some(path.to_str().unwrap()),
        )
        .unwrap();
        apply_plan(&first).unwrap();

        let second = build_plan(
            &target,
            "http://localhost:8080/v1",
            None,
            Some(path.to_str().unwrap()),
        )
        .unwrap();
        assert!(!second.changed);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn apply_plan_backs_up_the_previous_file() {
        let dir =
            std::env::temp_dir().join(format!("rp-cli-setup-test-backup-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("opencode.json");
        std::fs::write(&path, r#"{"existing": true}"#).unwrap();

        let target = json_target();
        let plan = build_plan(
            &target,
            "http://localhost:8080/v1",
            None,
            Some(path.to_str().unwrap()),
        )
        .unwrap();
        let backup = apply_plan(&plan).unwrap();
        assert!(backup.is_some());
        let backup_contents = std::fs::read_to_string(backup.unwrap()).unwrap();
        assert_eq!(backup_contents, r#"{"existing": true}"#);
        let new_contents = std::fs::read_to_string(&path).unwrap();
        assert!(new_contents.contains("baseURL"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn apply_plan_does_not_back_up_a_file_that_did_not_exist() {
        let dir =
            std::env::temp_dir().join(format!("rp-cli-setup-test-nobak-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("new.json");

        let target = json_target();
        let plan = build_plan(
            &target,
            "http://localhost:8080/v1",
            None,
            Some(path.to_str().unwrap()),
        )
        .unwrap();
        let backup = apply_plan(&plan).unwrap();
        assert!(backup.is_none());
        assert!(!std::path::Path::new(&format!("{}.bak", path.display())).exists());

        std::fs::remove_dir_all(&dir).ok();
    }

    fn toml_target() -> Target {
        Target {
            name: "test-toml".to_string(),
            description: "test".to_string(),
            config_path: "unused".to_string(),
            format: Format::Toml,
            fields: vec![FieldSpec {
                path: "model_providers.rp.base_url".to_string(),
                value: "{{base_url}}".to_string(),
                requires_api_key_env: false,
            }],
        }
    }

    #[test]
    fn build_plan_creates_a_new_toml_file_from_scratch() {
        let target = toml_target();
        let plan = build_plan(
            &target,
            "http://localhost:8080/v1",
            None,
            Some("/no/such/file"),
        )
        .unwrap();
        assert!(plan.changed);
        let parsed: toml::Value = plan.after.parse().unwrap();
        assert_eq!(
            parsed["model_providers"]["rp"]["base_url"].as_str(),
            Some("http://localhost:8080/v1")
        );
    }

    #[test]
    fn parse_targets_rejects_malformed_toml() {
        assert!(parse_targets("not valid toml [[[").is_err());
    }
}
