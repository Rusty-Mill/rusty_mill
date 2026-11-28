#![allow(clippy::unwrap_used)]

use super::*;

const BUILDER: &str = r#"
schema = 1
lens = "builder-rust"
role = "builder"

[tools]
allow = ["sandbox_*", "board_card_get", "github_*"]
deny  = ["github_merge_*"]

[workspace]
view = "worktree"
layers = ["rust"]

[skills]
packs = ["rust"]

[grants]
network = []
push = false
secrets = []
"#;

const CRITIC: &str = r#"
schema = 1
lens = "critic"
role = "critic"

[tools]
allow = ["sandbox_read", "sandbox_list"]
"#;

fn loaded() -> LensPolicies {
    LensPolicies::from_files([
        ("builder-rust.toml".to_owned(), BUILDER.as_bytes().to_vec()),
        ("critic.toml".to_owned(), CRITIC.as_bytes().to_vec()),
    ])
    .unwrap()
}

fn token(lens: &str, file: &str) -> serde_json::Value {
    serde_json::json!({ "lel.lens": lens, "lel.policy": policy_digest(file.as_bytes()) })
}

fn tool(policies: &LensPolicies, name: &str, claims: Option<&serde_json::Value>) -> bool {
    policies.permits(Subject::Tool(name), name, claims)
}

#[test]
fn the_tokens_lens_decides_which_tools_it_gets() {
    let p = loaded();
    let builder = token("builder-rust", BUILDER);
    assert!(tool(&p, "sandbox_exec", Some(&builder)));
    assert!(tool(&p, "github_create_pull_request", Some(&builder)));
    assert!(
        !tool(&p, "github_merge_pull_request", Some(&builder)),
        "deny wins"
    );
    assert!(!tool(&p, "board_card_move", Some(&builder)), "not allowed");

    let critic = token("critic", CRITIC);
    assert!(tool(&p, "sandbox_read", Some(&critic)));
    assert!(
        !tool(&p, "sandbox_exec", Some(&critic)),
        "a critic sees only read tools"
    );
}

#[test]
fn an_unknown_lens_a_wrong_digest_or_no_token_denies_everything() {
    let p = loaded();
    for claims in [
        Some(token("nobody", BUILDER)),
        Some(token("builder-rust", CRITIC)),
        Some(serde_json::json!({ "lel.lens": "builder-rust" })),
        Some(serde_json::json!({ "lel.policy": policy_digest(BUILDER.as_bytes()) })),
        Some(serde_json::json!({ "lel.lens": 7, "lel.policy": 7 })),
        None,
    ] {
        assert!(!tool(&p, "sandbox_read", claims.as_ref()), "{claims:?}");
    }
}

#[test]
fn prompts_and_resources_are_never_permitted() {
    let p = loaded();
    let builder = token("builder-rust", BUILDER);
    assert!(!p.permits(Subject::Prompt("sandbox_x"), "sandbox_x", Some(&builder)));
    assert!(!p.permits(
        Subject::Resource("sandbox+f:x"),
        "sandbox+f:x",
        Some(&builder)
    ));
}

#[test]
fn an_empty_allow_list_allows_nothing() {
    let file = "schema = 1\nlens = \"none\"\nrole = \"builder\"\n";
    let p = LensPolicies::from_files([("none.toml".to_owned(), file.as_bytes().to_vec())]).unwrap();
    assert!(!tool(&p, "sandbox_read", Some(&token("none", file))));
}

#[test]
fn the_digest_is_sha256_hex_of_the_bytes() {
    assert_eq!(
        policy_digest(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[test]
fn bad_files_are_refused_naming_the_file() {
    let cases = [
        ("x.toml", "lens = \"x\"\nrole = \"builder\"\n", "schema"),
        (
            "x.toml",
            "schema = 2\nlens = \"x\"\nrole = \"builder\"\n",
            "schema",
        ),
        ("x.toml", "schema = 1\nrole = \"builder\"\n", "lens"),
        (
            "y.toml",
            "schema = 1\nlens = \"x\"\nrole = \"builder\"\n",
            "file name",
        ),
        (
            "x.toml",
            "schema = 1\nlens = \"x\"\nrole = \"admin\"\n",
            "role",
        ),
        (
            "x.toml",
            "schema = 1\nlens = \"x\"\nrole = \"builder\"\ntols = 1\n",
            "unknown key `tols`",
        ),
        (
            "x.toml",
            "schema = 1\nlens = \"x\"\nrole = \"builder\"\ntools = 1\n",
            "`tools` must be a table",
        ),
        (
            "x.toml",
            "schema = 1\nlens = \"x\"\nrole = \"builder\"\n[tools]\nallow = \"*\"\n",
            "list of strings",
        ),
        (
            "x.toml",
            "schema = 1\nlens = \"x\"\nrole = \"builder\"\n[tools]\nallow = [1]\n",
            "list of strings",
        ),
        (
            "x.toml",
            "schema = 1\nlens = \"x\"\nrole = \"builder\"\ngrants = 1\n",
            "`grants` must be a table",
        ),
        (
            "x.toml",
            "schema = 1\nlens = \"x\"\nrole = \"builder\"\n[tools]\nallow = [\"*\"]\ndney = [\"github_merge_*\"]\n",
            "unknown key `tools.dney`",
        ),
        (
            "x.toml",
            "schema = 1\nlens = \"x\"\nrole = \"builder\"\n[tools]\nallow = [\"*\"]\n[tools.deny]\nall = true\n",
            "`tools.deny` must be a list of strings",
        ),
        (
            "x.toml",
            "schema = 1\nlens = \"x\"\nrole = \"builder\"\n[tools]\nallow = [\"*\"]\ndeny = [\"sandbox_rm\"]\ndeny = []\n",
            "duplicate key",
        ),
    ];
    for (file, text, reason) in cases {
        let err =
            LensPolicies::from_files([(file.to_owned(), text.as_bytes().to_vec())]).unwrap_err();
        assert_eq!(err.file, file);
        assert!(err.reason.contains(reason), "{text:?}: {err}");
    }
}

#[test]
fn an_escaped_deny_entry_still_denies() {
    let text = "schema = 1\nlens = \"x\"\nrole = \"builder\"\n[tools]\nallow = [\"*\"]\n\
                deny = [\"\\u0073andbox_rm\"]\n";
    let policies =
        LensPolicies::from_files([("x.toml".to_owned(), text.as_bytes().to_vec())]).unwrap();
    let claims = token("x", text);
    assert!(!tool(&policies, "sandbox_rm", Some(&claims)));
    assert!(tool(&policies, "sandbox_read", Some(&claims)));
}

#[test]
fn opaque_sections_may_hold_any_keys() {
    let text = "schema = 1\nlens = \"x\"\nrole = \"builder\"\n[tools]\nallow = [\"a\"]\n\
                [workspace]\nview = \"worktree\"\nanything = 1\n[skills]\nx = [\"y\"]\n\
                [grants]\nz = { w = 1 }\n";
    let loaded = LensPolicies::from_files([("x.toml".to_owned(), text.as_bytes().to_vec())]);
    assert!(loaded.is_ok(), "{loaded:?}");
}

#[test]
fn a_lens_defined_twice_is_refused() {
    let err = LensPolicies::from_files([
        ("critic.toml".to_owned(), CRITIC.as_bytes().to_vec()),
        ("critic.toml".to_owned(), CRITIC.as_bytes().to_vec()),
    ])
    .unwrap_err();
    assert!(err.reason.contains("twice"), "{err}");
}
