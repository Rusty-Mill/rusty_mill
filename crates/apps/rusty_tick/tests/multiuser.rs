//! Several users on one server (ADR-0002, step 3), through the backend with no
//! sockets: isolation, refusals that look alike, live revocation, eviction and
//! the directory locks.

use rusty_http::Method;
use rusty_json::Value;
use rusty_multimodal_db_engine::dir_lock::{DirLock, DirLockError};
use rusty_tick::api::Request;
use rusty_tick::backend::{Backend, BackendError, LOCK_FILE, USERS_DIR, USERS_FILE};
use rusty_tick::service::system_clock;
use rusty_tick::users::{Registry, UserKey};
use std::num::NonZeroUsize;
use std::path::Path;
use std::time::Duration;

/// Every store starts with one permanent list.
const INBOX: &str = "Inbox";

fn key(name: &str) -> UserKey {
    UserKey::parse(name).unwrap()
}

/// A data directory with `alice` and `bob`, each with one token.
struct Fixture {
    dir: tempfile::TempDir,
    registry: Registry,
    alice: String,
    bob: String,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let mut registry = Registry::default();
        for name in ["alice", "bob"] {
            registry.add_user(&key(name)).unwrap();
        }
        let alice = registry.add_token(&key("alice"), "phone", 1).unwrap();
        let bob = registry.add_token(&key("bob"), "laptop", 1).unwrap();
        registry.save(&dir.path().join(USERS_FILE)).unwrap();
        Self {
            dir,
            registry,
            alice,
            bob,
        }
    }

    fn backend(&self, capacity: usize) -> Backend {
        Backend::multi(
            self.dir.path(),
            NonZeroUsize::new(capacity).unwrap(),
            Box::new(system_clock),
        )
        .unwrap()
    }

    fn save(&self) {
        self.registry
            .save(&self.dir.path().join(USERS_FILE))
            .unwrap();
    }
}

fn send(
    backend: &mut Backend,
    method: Method,
    target: &str,
    body: &str,
    token: Option<&str>,
) -> (u16, Vec<u8>) {
    let header = token.map(|t| format!("Bearer {t}"));
    let request = Request {
        method: &method,
        target,
        authorization: header.as_deref(),
        if_match: None,
        body: body.as_bytes(),
    };
    let response = backend.handle(&request);
    (response.status.as_u16(), response.body)
}

fn json(body: &[u8]) -> Value {
    rusty_json::from_slice(body).expect("responses are JSON")
}

fn create_list(backend: &mut Backend, token: &str, name: &str) -> String {
    let (status, body) = send(
        backend,
        Method::Post,
        "/api/v1/lists",
        &format!(r#"{{"name":"{name}"}}"#),
        Some(token),
    );
    assert_eq!(status, 201);
    json(&body)["id"].as_str().unwrap().to_string()
}

fn list_names(backend: &mut Backend, token: &str) -> Vec<String> {
    let (status, body) = send(backend, Method::Get, "/api/v1/lists", "", Some(token));
    assert_eq!(status, 200);
    json(&body)["lists"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l["name"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn users_see_only_their_own_lists() {
    let f = Fixture::new();
    let mut backend = f.backend(4);
    let inbox = create_list(&mut backend, &f.alice, "Alice's inbox");
    create_list(&mut backend, &f.bob, "Bob's chores");

    assert_eq!(list_names(&mut backend, &f.alice), [INBOX, "Alice's inbox"]);
    assert_eq!(list_names(&mut backend, &f.bob), [INBOX, "Bob's chores"]);
    // Bob cannot reach Alice's list by its id either.
    let (status, _) = send(
        &mut backend,
        Method::Get,
        &format!("/api/v1/lists/{inbox}"),
        "",
        Some(&f.bob),
    );
    assert_eq!(status, 404);
    // Each user's data is in their own directory.
    let users = f.dir.path().join(USERS_DIR);
    assert!(users.join("alice").is_dir() && users.join("bob").is_dir());
}

#[test]
fn every_refusal_is_the_same_401() {
    let mut f = Fixture::new();
    f.registry.set_disabled(&key("bob"), true).unwrap();
    f.save();
    let mut backend = f.backend(4);
    let (_, alices_secret) = f.alice.split_once('.').unwrap();
    let wrong_secret = format!("alice.{}", "B".repeat(alices_secret.len()));
    let cases: Vec<Option<String>> = vec![
        None,
        Some(String::new()),
        Some(f.bob.clone()),                         // a disabled user
        Some(format!("nobody.{alices_secret}")),     // an unknown user
        Some(wrong_secret),                          // the right shape, wrong secret
        Some("not-a-token-at-all-0123".to_string()), // a single-user style token
        Some(alices_secret.to_string()),             // the secret without its user
    ];
    let mut bodies = Vec::new();
    for case in &cases {
        let (status, body) = send(
            &mut backend,
            Method::Get,
            "/api/v1/lists",
            "",
            case.as_deref(),
        );
        assert_eq!(status, 401, "{case:?}");
        bodies.push(body);
    }
    assert!(bodies.windows(2).all(|pair| pair[0] == pair[1]));
    // Nothing was opened for any of them.
    assert_eq!(backend.open_stores(), 0);
}

#[test]
fn a_scheme_other_than_bearer_is_refused() {
    let f = Fixture::new();
    let mut backend = f.backend(4);
    let request = Request {
        method: &Method::Get,
        target: "/api/v1/lists",
        authorization: Some(&format!("Basic {}", f.alice)),
        if_match: None,
        body: b"",
    };
    assert_eq!(backend.handle(&request).status.as_u16(), 401);
}

#[test]
fn health_needs_no_token_and_opens_no_store() {
    let f = Fixture::new();
    let mut backend = f.backend(4);
    let (status, _) = send(&mut backend, Method::Get, "/health", "", None);
    assert_eq!(status, 200);
    assert_eq!(backend.open_stores(), 0);
}

#[test]
fn a_revoked_token_stops_working_without_a_restart() {
    let mut f = Fixture::new();
    let mut backend = f.backend(4);
    assert_eq!(list_names(&mut backend, &f.alice), [INBOX]);

    let id = f.registry.user(&key("alice")).unwrap().tokens[0].id.clone();
    f.registry.revoke(&key("alice"), &id).unwrap();
    f.save();
    // The running server looks at the file at most once a second.
    std::thread::sleep(Duration::from_millis(1_200));
    let (status, _) = send(
        &mut backend,
        Method::Get,
        "/api/v1/lists",
        "",
        Some(&f.alice),
    );
    assert_eq!(status, 401);
    assert_eq!(list_names(&mut backend, &f.bob), [INBOX]);
}

#[test]
fn a_user_evicted_from_the_pool_comes_back_with_their_data() {
    let f = Fixture::new();
    let mut backend = f.backend(1);
    create_list(&mut backend, &f.alice, "Kept");
    assert_eq!(backend.open_stores(), 1);
    create_list(&mut backend, &f.bob, "Other"); // closes alice's store
    assert_eq!(backend.open_stores(), 1);
    assert_eq!(list_names(&mut backend, &f.alice), [INBOX, "Kept"]);
    assert_eq!(backend.open_stores(), 1);
}

#[test]
fn a_directory_held_by_another_process_is_a_503_for_that_user_only() {
    let f = Fixture::new();
    let mut backend = f.backend(4);
    let _held = DirLock::acquire(&f.dir.path().join(USERS_DIR).join("alice"), LOCK_FILE).unwrap();
    let (status, body) = send(
        &mut backend,
        Method::Get,
        "/api/v1/lists",
        "",
        Some(&f.alice),
    );
    assert_eq!(status, 503);
    assert_eq!(json(&body)["error"]["code"], "unavailable");
    assert_eq!(list_names(&mut backend, &f.bob), [INBOX]);
}

#[test]
fn multi_user_mode_is_chosen_by_the_users_file() {
    let dir = tempfile::tempdir().unwrap();
    assert!(!Backend::is_multi_user(dir.path()));
    let missing = Backend::multi(
        dir.path(),
        NonZeroUsize::new(1).unwrap(),
        Box::new(system_clock),
    );
    assert!(matches!(missing, Err(BackendError::Users(_))));

    Registry::default()
        .save(&dir.path().join(USERS_FILE))
        .unwrap();
    assert!(Backend::is_multi_user(dir.path()));
}

#[test]
fn single_user_mode_locks_its_directory() {
    let dir = tempfile::tempdir().unwrap();
    let token = "test-token-0123456789";
    let first = Backend::single(dir.path(), token.into(), system_clock()).unwrap();
    let second = Backend::single(dir.path(), token.into(), system_clock());
    assert!(matches!(
        second,
        Err(BackendError::Lock(DirLockError::Held(_)))
    ));
    drop(first);
    Backend::single(dir.path(), token.into(), system_clock()).unwrap();
}

#[test]
fn single_user_mode_keeps_its_shared_token_and_its_directory() {
    let dir = tempfile::tempdir().unwrap();
    let token = "test-token-0123456789";
    let mut backend = Backend::single(dir.path(), token.into(), system_clock()).unwrap();
    assert!(matches!(
        Backend::single(Path::new("unused"), "short".into(), system_clock()),
        Err(BackendError::Token(_))
    ));
    create_list(&mut backend, token, "Inbox 2");
    assert_eq!(list_names(&mut backend, token), [INBOX, "Inbox 2"]);
    let (status, _) = send(
        &mut backend,
        Method::Get,
        "/api/v1/lists",
        "",
        Some("wrong"),
    );
    assert_eq!(status, 401);
    // The store is the data directory itself, not a users/ subdirectory.
    assert!(dir.path().join("lists.mmap").exists());
    assert!(dir.path().join("tasks.mmap").exists());
    assert!(!dir.path().join(USERS_DIR).exists());
}
