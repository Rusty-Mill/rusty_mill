use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt;

use orch_jail::spec::JailSpec;
use orch_token::AUDIENCE;
use rusty_json::Value;

use orch_jail::spec::Resolved;

use super::*;
use crate::builder::live::{jail_file, JailInputs};

/// orch-token's throwaway test key; signs nothing real.
const PEM: &[u8] = include_bytes!("../../../../orch-token/testdata/test-only-p256.pem");
const BUILDER: &str = "schema = 1\nlens = \"builder-rust\"\nrole = \"builder\"\n";
const SEAT: Seat<'static> = Seat {
    board: "bbp_factory",
    task: "t-1382",
    claim: "42",
    n: 3,
};

fn signer() -> Signer {
    let key = SigningKey::from_pem(PEM, "orch-1").expect("test key");
    let policy = policy("builder-rust.toml", BUILDER.as_bytes()).expect("policy");
    Signer::new(key, policy, "baileyai")
}

/// The real path, as the launcher's resolver sees it.
fn canonical(p: &Path) -> Result<Resolved, String> {
    let real = std::fs::canonicalize(p).map_err(|e| format!("{}: {e}", p.display()))?;
    Ok(Resolved {
        handle: real.clone(),
        real,
    })
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("orch-token-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch");
    dir
}

/// The payload, read without checking the signature (orch-token's own tests
/// check that); enough to see what the claims say.
fn payload(token: &str) -> Value {
    let body = token.split('.').nth(1).expect("three parts");
    let bytes = rusty_base64::decode_url_safe(body).expect("base64url");
    Value::parse(std::str::from_utf8(&bytes).expect("utf-8")).expect("json")
}

fn text<'a>(claims: &'a Value, key: &str) -> &'a str {
    claims.get(key).and_then(Value::as_str).unwrap_or_default()
}

#[test]
fn the_token_names_the_seat_lens_and_policy() {
    let minted =
        mint_for(&signer(), SEAT, [7; 16], 1_000, Duration::from_secs(2_700)).expect("mints");
    let claims = payload(&minted.token);
    assert_eq!(text(&claims, "iss"), "lel-orch@baileyai");
    assert_eq!(text(&claims, "aud"), AUDIENCE);
    assert_eq!(text(&claims, "sub"), "bbp_factory/42/builder-rust/3");
    assert_eq!(text(&claims, "lel.task"), "t-1382");
    assert_eq!(text(&claims, "lel.role"), "builder");
    assert_eq!(text(&claims, "lel.lens"), "builder-rust");
    assert_eq!(
        text(&claims, "lel.policy"),
        policy_digest(BUILDER.as_bytes()).as_str()
    );
    assert_eq!(
        claims.get("exp").and_then(Value::as_u64),
        Some(1_000 + 2_700 + 300)
    );
    assert_eq!(minted.sub, "bbp_factory/42/builder-rust/3");
    assert_eq!(minted.jti, "07".repeat(16));
}

#[test]
fn a_policy_whose_lens_is_not_its_file_name_is_refused() {
    let err = policy("builder.toml", BUILDER.as_bytes()).expect_err("refused");
    assert!(err.contains("must match the file name"), "{err}");
}

#[test]
fn a_critic_policy_is_refused_for_a_builder() {
    let critic = "schema = 1\nlens = \"critic-rust\"\nrole = \"critic\"\n";
    let err = policy("critic-rust.toml", critic.as_bytes()).expect_err("refused");
    assert!(err.contains("role = \"builder\""), "{err}");
}

#[test]
fn a_policy_without_a_lens_or_not_utf8_is_refused() {
    assert!(policy("x.toml", b"schema = 1\nrole = \"builder\"\n").is_err());
    assert!(policy("x.toml", &[0xff, 0xfe]).is_err());
}

fn mode(p: &Path) -> u32 {
    std::fs::metadata(p).expect("meta").permissions().mode() & 0o777
}

#[test]
fn acquire_makes_a_0600_file_in_a_0700_dir_and_drop_removes_both() {
    let state = scratch("acquire");
    let file = path(&state, "42", 3);
    assert_eq!(file, state.join("tokens/42/3/token"));
    let dir = file.parent().expect("dir").to_owned();
    {
        let held = acquire(&file).expect("acquires");
        held.write("secret").expect("writes");
        assert_eq!(std::fs::read_to_string(&file).expect("reads"), "secret");
        assert_eq!(mode(&file), 0o600);
        assert_eq!(mode(&dir), 0o700);
    }
    assert!(
        !file.exists() && !dir.exists(),
        "drop removes what acquire made"
    );
    assert!(
        state.join("tokens/42").exists(),
        "the claim dir is shared, kept"
    );
    let _ = std::fs::remove_dir_all(&state);
}

#[test]
fn an_existing_session_dir_is_refused_and_left_untouched() {
    let state = scratch("occupied");
    let file = path(&state, "42", 3);
    let dir = file.parent().expect("dir").to_owned();
    std::fs::create_dir_all(&dir).expect("earlier run's dir");
    std::fs::write(&file, "theirs").expect("earlier run's token");
    let err = acquire(&file).expect_err("refused");
    assert!(err.contains("earlier run"), "{err}");
    assert_eq!(
        std::fs::read_to_string(&file).expect("still there"),
        "theirs"
    );
    let _ = std::fs::remove_dir_all(&state);
}

#[test]
fn a_failed_create_leaves_nothing_and_touches_nothing_else() {
    let state = scratch("create-fails");
    std::fs::create_dir_all(state.join("tokens")).expect("tokens");
    // The claim "dir" is a file, so neither directory can be made.
    std::fs::write(state.join("tokens/42"), "not a dir").expect("blocker");
    let err = acquire(&path(&state, "42", 3)).expect_err("refused");
    assert!(err.contains("tokens/42"), "{err}");
    assert_eq!(
        std::fs::read_to_string(state.join("tokens/42")).expect("kept"),
        "not a dir"
    );
    let _ = std::fs::remove_dir_all(&state);
}

#[test]
fn a_failed_write_still_cleans_up_on_drop() {
    let state = scratch("write-fails");
    let file = path(&state, "42", 3);
    let dir = file.parent().expect("dir").to_owned();
    {
        let held = acquire(&file).expect("acquires");
        std::fs::remove_file(&file).expect("vanish under the writer");
        std::fs::create_dir(&file).expect("a dir where the file was");
        assert!(held.write("t").is_err(), "writing into a dir fails");
        std::fs::remove_dir(&file).expect("undo the fixture");
    }
    assert!(!dir.exists(), "the guard removed its directory");
    let _ = std::fs::remove_dir_all(&state);
}

#[test]
fn the_jail_file_with_the_slot_resolves_and_keeps_the_token_out_of_every_root() {
    let state = scratch("jail");
    let worktree = state.join("wt/42/3");
    let run = state.join("builders/42/3");
    std::fs::create_dir_all(&worktree).expect("wt");
    std::fs::create_dir_all(&run).expect("run");
    let home = state.join("home");
    std::fs::create_dir_all(home.join(".claude")).expect("home");
    let file = path(&state, "42", 3);
    let _held = acquire(&file).expect("acquires");
    let jail = jail_file(&JailInputs {
        cgroup: Path::new("/sys/fs/cgroup/lel-store-ab/builder-42-3-10"),
        status: &run.join("status"),
        worktree: &worktree,
        home: &home,
        extra: &[],
        cpu_secs: 60,
        env: BTreeMap::new(),
        token: Some(slot(&file)),
    });
    let token = jail.token.clone().expect("slot filled");
    assert_eq!(token.var, VAR);
    assert_eq!(token.file, file.display().to_string());
    let spec = JailSpec::validate(jail).expect("lel-jail accepts it");
    spec.resolve(canonical)
        .expect("token file resolves outside every root");
    let _ = std::fs::remove_dir_all(&state);
}

#[test]
fn a_read_root_over_the_state_dir_refuses_the_jail() {
    let state = scratch("reach");
    let worktree = state.join("wt/42/3");
    std::fs::create_dir_all(&worktree).expect("wt");
    let file = path(&state, "42", 3);
    let _held = acquire(&file).expect("acquires");
    let extra = [state.clone()];
    let jail = jail_file(&JailInputs {
        cgroup: Path::new("/sys/fs/cgroup/lel-store-ab/builder-42-3-10"),
        status: &state.join("status"),
        worktree: &worktree,
        home: Path::new("/nonexistent-home"),
        extra: &extra,
        cpu_secs: 60,
        env: BTreeMap::new(),
        token: Some(slot(&file)),
    });
    assert!(
        JailSpec::validate(jail).is_err(),
        "a --read over the token is refused"
    );
    let _ = std::fs::remove_dir_all(&state);
}

#[test]
fn jti_is_random() {
    assert_ne!(jti().expect("jti"), jti().expect("jti"));
}
