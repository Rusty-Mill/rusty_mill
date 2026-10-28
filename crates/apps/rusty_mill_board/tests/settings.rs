//! Settings over a real socket: who may change what, that a change takes effect at once and is
//! written to its file (secrets owner-only), and the refusals that keep the board usable.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::sync::Arc;

use rusty_mcp_server::{HttpConfig, HttpHandler};
use rusty_mill_board::api::Shared;
use rusty_mill_board::board::{Board, Roles};
use rusty_mill_board::domain::MoveRules;
use rusty_mill_board::identities::Credentials;
use rusty_mill_board::mcp;
use rusty_mill_board::projects::Projects;
use rusty_mill_board::settings::{
    Files, ServerInfo, Settings, load_github, load_roles, load_rules, open_credentials,
};
use rusty_mill_board::store::{MemStore, Store};
use rusty_mill_board::syncer::Syncer;
use rusty_mill_board::web::{Auth, Web};
use serde_json::{Value, json};

const ISSUER: &str = "http://localhost:8787";

struct Fixture {
    addr: SocketAddr,
    dir: PathBuf,
    admin: String,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// A board wired as `main` wires it, settings kept in a fresh directory.
fn start(name: &str) -> Fixture {
    start_with(name, false)
}

/// [`start`], with OAuth sign-in on when `oauth`, its grants beside the settings.
fn start_with(name: &str, oauth: bool) -> Fixture {
    let dir = std::env::temp_dir().join(format!("mill_settings_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let files = Files::in_dir(&dir);
    let roles = Roles::default();
    let projects = Projects::default();
    let board: Arc<Shared> = Arc::new(
        Board::new(
            Box::new(MemStore::default()) as Box<dyn Store>,
            roles.clone(),
        )
        .with_projects(projects.clone()),
    );
    let (creds, first) = open_credentials(
        files.identities.as_deref().expect("path"),
        &roles,
        &projects,
    )
    .expect("credentials");
    let creds = creds.shared();
    let settings = Arc::new(
        Settings::new(
            board.clone(),
            files,
            Some(Arc::clone(&creds)),
            Syncer::new(board.clone()),
            None,
        )
        .with_server(ServerInfo {
            tick: None,
            issuer: oauth.then(|| ISSUER.to_owned()),
            hosts: vec!["127.0.0.1".into()],
        }),
    );
    let config = HttpConfig {
        allowed_hosts: vec!["127.0.0.1".into()],
        ..HttpConfig::default()
    };
    let path = config.path.clone();
    let mcp = HttpHandler::new(mcp::server(board.clone()).expect("server"), config);
    let web = Web::new(
        board,
        mcp,
        path,
        vec!["127.0.0.1".into()],
        Auth::Credentials(creds),
    )
    .with_settings(settings);
    let web = if oauth {
        let issuer = rusty_oauth_server::validate_issuer(ISSUER).expect("issuer");
        web.with_oauth(issuer, dir.join("oauth.json"))
            .expect("oauth")
    } else {
        web
    };
    let server =
        rusty_serve::Server::bind_shared("127.0.0.1:0".parse().expect("addr"), web).expect("bind");
    let addr = server.local_addr().expect("addr");
    std::thread::spawn(move || {
        let _ = server.run();
    });
    Fixture {
        addr,
        dir,
        admin: first.expect("a fresh directory gets a first admin"),
    }
}

fn call(addr: SocketAddr, token: &str, op: &str, body: &Value) -> (u16, Value, String) {
    let body = body.to_string();
    let raw = format!(
        "POST /api/{op} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\nContent-Type: application/json\r\nAuthorization: Bearer {token}\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    let mut sock = TcpStream::connect(addr).expect("connect");
    sock.write_all(raw.as_bytes()).expect("write");
    let mut out = String::new();
    sock.read_to_string(&mut out).expect("read");
    let status = out
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .expect("status");
    let text = out.split("\r\n\r\n").nth(1).unwrap_or("").to_owned();
    (
        status,
        serde_json::from_str(&text).unwrap_or(Value::Null),
        text,
    )
}

impl Fixture {
    fn admin(&self, op: &str, body: &Value) -> (u16, Value, String) {
        call(self.addr, &self.admin, op, body)
    }

    fn file(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    fn add_agent(&self, name: &str, role: &str) -> String {
        let (status, out, text) = self.admin(
            "settings_agent_add",
            &json!({"agent": {"name": name, "role": role}}),
        );
        assert_eq!(status, 200, "{text}");
        out["token"].as_str().expect("token").to_owned()
    }
}

#[cfg(unix)]
fn mode(path: &std::path::Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).expect("file").permissions().mode() & 0o777
}

#[test]
fn a_new_directory_gets_a_first_admin_whose_token_works_and_is_kept_private() {
    let f = start("bootstrap");
    let (status, out, _) = f.admin("settings_get", &json!({}));
    assert_eq!(status, 200);
    assert_eq!(
        out["agents"],
        json!([{"actor": "admin", "role": "human", "projects": null}])
    );
    #[cfg(unix)]
    assert_eq!(
        mode(&f.file("identities.json")),
        0o600,
        "the credentials file is owner-only"
    );
    // A second start finds the file and creates nothing.
    let roles = Roles::default();
    let (_, again) =
        open_credentials(&f.file("identities.json"), &roles, &Projects::default()).expect("open");
    assert_eq!(again, None);
}

#[test]
fn only_an_admin_may_change_settings() {
    let f = start("gate");
    let builder = f.add_agent("claude", "builder");
    for op in [
        "settings_get",
        "settings_agent_add",
        "settings_agent_update",
        "settings_agent_remove",
        "settings_agent_rotate",
        "settings_projects_set",
        "settings_roles_set",
        "settings_github_set",
        "settings_github_clear",
        "settings_rules_set",
    ] {
        let (status, _, text) = call(f.addr, &builder, op, &json!({}));
        assert_eq!(status, 403, "{op}: {text}");
        assert_eq!(
            call(f.addr, "no-such-token-no-such", op, &json!({})).0,
            401,
            "{op}"
        );
    }
}

#[test]
fn a_new_agent_works_at_once_and_its_token_is_never_shown_again() {
    let f = start("agents");
    let token = f.add_agent("claude", "builder");
    let (status, me, _) = call(f.addr, &token, "board_whoami", &json!({}));
    assert_eq!(
        (status, me["actor"].as_str(), me["role"].as_str()),
        (200, Some("claude"), Some("builder"))
    );

    // The file has it, owner-only; the settings view does not.
    let creds = Credentials::load(
        &f.file("identities.json"),
        &Roles::default(),
        &Projects::default(),
    )
    .expect("load");
    assert_eq!(
        creds.identify(&token).map(|i| i.actor.as_str()),
        Some("claude")
    );
    #[cfg(unix)]
    assert_eq!(mode(&f.file("identities.json")), 0o600);
    let (_, _, listing) = f.admin("settings_get", &json!({}));
    assert!(
        listing.contains("claude") && !listing.contains(&token) && !listing.contains(&f.admin),
        "{listing}"
    );

    // Rotating replaces the token; removing ends the credential.
    let (status, out, _) = f.admin("settings_agent_rotate", &json!({"name": "claude"}));
    assert_eq!(status, 200);
    let rotated = out["token"].as_str().expect("token").to_owned();
    assert_ne!(rotated, token);
    assert_eq!(
        call(f.addr, &token, "board_whoami", &json!({})).0,
        401,
        "the old token stops working"
    );
    assert_eq!(call(f.addr, &rotated, "board_whoami", &json!({})).0, 200);
    assert_eq!(
        f.admin("settings_agent_remove", &json!({"name": "claude"}))
            .0,
        200
    );
    assert_eq!(call(f.addr, &rotated, "board_whoami", &json!({})).0, 401);
    let reread = Credentials::load(
        &f.file("identities.json"),
        &Roles::default(),
        &Projects::default(),
    )
    .expect("load");
    assert!(!reread.contains("claude"));
}

#[test]
fn bad_agents_are_refused() {
    let f = start("badagents");
    f.add_agent("claude", "builder");
    for (body, why) in [
        (
            json!({"agent": {"name": "claude", "role": "builder"}}),
            "already",
        ),
        (
            json!({"agent": {"name": "has space", "role": "builder"}}),
            "agent name",
        ),
        (
            json!({"agent": {"name": "", "role": "builder"}}),
            "agent name",
        ),
        (json!({"agent": {"name": "x", "role": "ghost"}}), "no role"),
        (
            json!({"agent": {"name": "x", "role": "builder", "projects": ["nope"]}}),
            "no project",
        ),
        (
            json!({"agent": {"name": "x", "role": "builder", "projects": []}}),
            "at least one",
        ),
        (json!({"agent": {"role": "builder"}}), "invalid arguments"),
    ] {
        let (status, _, text) = f.admin("settings_agent_add", &body);
        assert_eq!(status, 422, "{body}: {text}");
        assert!(text.contains(why), "{body}: {text}");
    }
    assert_eq!(
        f.admin("settings_agent_remove", &json!({"name": "ghost"}))
            .0,
        422
    );
}

#[test]
fn nothing_may_leave_the_board_without_an_admin() {
    let f = start("lockout");
    // The only admin cannot be removed or demoted.
    for (op, body) in [
        ("settings_agent_remove", json!({"name": "admin"})),
        (
            "settings_agent_update",
            json!({"agent": {"name": "admin", "role": "builder"}}),
        ),
    ] {
        let (status, _, text) = f.admin(op, &body);
        assert_eq!(status, 422, "{op}: {text}");
        assert!(text.contains("no agent able to change settings"), "{text}");
    }
    // Nor can the roles be edited so that no role holds admin, or so that an agent's role vanishes.
    let (_, out, _) = f.admin("settings_get", &json!({}));
    let mut roles = out["roles"].clone();
    roles["human"] = json!(["triage", "build", "qa", "review", "merge"]);
    let (status, _, text) = f.admin("settings_roles_set", &json!({"roles": roles}));
    assert_eq!(
        (status, text.contains("admin capability")),
        (422, true),
        "{text}"
    );
    let mut without_human = out["roles"].clone();
    without_human
        .as_object_mut()
        .expect("roles")
        .remove("human");
    without_human["boss"] = json!(["admin"]);
    let (status, _, text) = f.admin("settings_roles_set", &json!({"roles": without_human}));
    assert_eq!(
        (status, text.contains("would no longer exist")),
        (422, true),
        "{text}"
    );

    // A second admin makes the first removable.
    let deputy = f.add_agent("deputy", "human");
    assert_eq!(
        f.admin("settings_agent_remove", &json!({"name": "admin"}))
            .0,
        200
    );
    assert_eq!(call(f.addr, &f.admin, "board_whoami", &json!({})).0, 401);
    let (status, _, text) = call(
        f.addr,
        &deputy,
        "settings_agent_remove",
        &json!({"name": "deputy"}),
    );
    assert_eq!(status, 422, "the last one again: {text}");
}

#[test]
fn roles_are_edited_live_and_written_to_their_file() {
    let f = start("roles");
    let (_, out, _) = f.admin("settings_get", &json!({}));
    assert_eq!(
        out["capabilities"],
        json!([
            "triage", "build", "qa", "review", "merge", "close", "project", "admin"
        ])
    );
    let mut roles = out["roles"].clone();
    roles["tester"] = json!(["qa"]);
    let (status, _, text) = f.admin("settings_roles_set", &json!({"roles": roles}));
    assert_eq!(status, 200, "{text}");
    // The new role can be given to an agent straight away, and is on disk.
    let token = f.add_agent("gemini", "tester");
    let (_, me, _) = call(f.addr, &token, "board_whoami", &json!({}));
    assert_eq!(me["capabilities"], json!(["qa"]));
    assert!(
        load_roles(&f.file("roles.json"))
            .expect("file")
            .0
            .contains_key("tester")
    );
    // A role an agent still has cannot be dropped; a badly named one is refused.
    let mut without = roles.clone();
    without.as_object_mut().expect("roles").remove("tester");
    let (status, _, text) = f.admin("settings_roles_set", &json!({"roles": without}));
    assert_eq!((status, text.contains("gemini")), (422, true), "{text}");
    roles["Bad Name"] = json!(["qa"]);
    assert_eq!(
        f.admin("settings_roles_set", &json!({"roles": roles})).0,
        422
    );
}

#[test]
fn repositories_and_projects_are_edited_live_and_written_to_their_file() {
    let f = start("projects");
    let repos = json!({
        "rusty_mill": {"rocket_league": ["crates/apps/rocket_league"]},
        "super_board": {"skills": ["skills"]},
    });
    let (status, _, text) = f.admin("settings_projects_set", &json!({"repos": repos}));
    assert_eq!(status, 200, "{text}");
    let on_disk = Projects::load(&f.file("projects.json")).expect("file");
    assert_eq!(on_disk.repo_of("skills"), Some("super_board"));
    // Cards can be made in the new project, and an agent can be limited to it.
    let (status, card, text) = f.admin(
        "card_add",
        &json!({"title": "t", "project": "rocket_league"}),
    );
    assert_eq!(status, 200, "{text}");
    // Dropping a project that has cards is refused, and nothing changes on disk.
    let before = std::fs::read_to_string(f.file("projects.json")).expect("read");
    let (status, _, text) = f.admin(
        "settings_projects_set",
        &json!({"repos": {"super_board": {"skills": ["skills"]}}}),
    );
    assert_eq!(
        (status, text.contains("still has cards")),
        (422, true),
        "{text}"
    );
    assert_eq!(
        std::fs::read_to_string(f.file("projects.json")).expect("read"),
        before
    );

    // An agent limited to a project makes dropping that project refused too.
    let limited = {
        let (status, out, text) = f.admin(
            "settings_agent_add",
            &json!({"agent": {"name": "rl-bot", "role": "builder", "projects": ["rocket_league"]}}),
        );
        assert_eq!(status, 200, "{text}");
        out["token"].as_str().expect("token").to_owned()
    };
    assert_eq!(
        call(
            f.addr,
            &limited,
            "card_get",
            &json!({"id": card["card"]["id"]})
        )
        .0,
        200
    );

    // So is dropping one an agent is limited to.
    f.admin(
        "card_move",
        &json!({"id": card["card"]["id"], "to": "ready"}),
    );
    let (status, _, text) = f.admin(
        "card_assign",
        &json!({"id": card["card"]["id"], "project": "rusty_mill"}),
    );
    assert_eq!(status, 200, "{text}");
    let (status, _, text) = f.admin(
        "settings_projects_set",
        &json!({"repos": {"rusty_mill": {}}}),
    );
    assert_eq!((status, text.contains("rl-bot")), (422, true), "{text}");
    // Invalid shapes are refused.
    for bad in [
        json!({"a": {"x": ["/abs"]}}),
        json!({"a": {"x": ["p"]}, "b": {"x": ["q"]}}),
        json!({"Bad": {}}),
    ] {
        assert_eq!(
            f.admin("settings_projects_set", &json!({"repos": bad})).0,
            422,
            "{bad}"
        );
    }
}

#[test]
fn the_github_sync_is_configured_without_ever_showing_its_token() {
    let f = start("github");
    let secret = "ghp_secret_token_value_0123456789";
    // An unusable repo name or interval is refused before anything is saved.
    for body in [
        json!({"repo": "../x", "token": secret}),
        json!({"repo": "owner/name"}),
        json!({"repo": "owner/name", "token": secret, "interval_secs": 1}),
        json!({"repo": "owner/name", "token": secret, "api": "ftp://x"}),
    ] {
        assert_eq!(f.admin("settings_github_set", &body).0, 422, "{body}");
    }
    assert!(!f.file("github.json").exists());

    // The API base is unreachable on purpose: the sync fails quietly and keeps trying.
    let (status, out, text) = f.admin(
        "settings_github_set",
        &json!({"repo": "owner/name", "token": secret, "api": "http://127.0.0.1:9", "interval_secs": 30}),
    );
    assert_eq!(status, 200, "{text}");
    assert_eq!(
        (
            out["repo"].as_str(),
            out["has_token"].as_bool(),
            out["running"].as_bool()
        ),
        (Some("owner/name"), Some(true), Some(true))
    );
    assert!(!text.contains(secret), "{text}");
    let (_, _, listing) = f.admin("settings_get", &json!({}));
    assert!(
        !listing.contains(secret) && listing.contains("owner/name"),
        "{listing}"
    );
    #[cfg(unix)]
    assert_eq!(
        mode(&f.file("github.json")),
        0o600,
        "the token file is owner-only"
    );
    assert_eq!(
        load_github(&f.file("github.json"))
            .expect("file")
            .expect("some")
            .token,
        secret
    );

    // A blank token keeps the stored one; clearing stops the sync and removes the file.
    let (status, out, text) = f.admin(
        "settings_github_set",
        &json!({"repo": "owner/other", "token": ""}),
    );
    assert_eq!(
        (status, out["repo"].as_str()),
        (200, Some("owner/other")),
        "{text}"
    );
    assert_eq!(
        load_github(&f.file("github.json"))
            .expect("file")
            .expect("some")
            .token,
        secret
    );
    assert_eq!(f.admin("settings_github_clear", &json!({})).0, 200);
    assert!(!f.file("github.json").exists());
    let (_, out, _) = f.admin("settings_get", &json!({}));
    assert_eq!(out["github"], Value::Null);
}

#[test]
fn without_files_settings_can_be_read_but_not_changed() {
    let board: Arc<Shared> = Arc::new(Board::new(Box::new(MemStore::default()), Roles::default()));
    let settings = Settings::read_only(board);
    let human = rusty_mill_board::api::Identity {
        actor: "nano".into(),
        role: "human".into(),
        projects: None,
    };
    let get = settings
        .call("settings_get", Some(&human), json!({}))
        .expect("op")
        .expect("ok");
    assert_eq!(
        get["files"],
        json!({"agents": false, "projects": false, "roles": false, "github": false, "rules": false})
    );
    for (op, body) in [
        (
            "settings_agent_add",
            json!({"agent": {"name": "x", "role": "builder"}}),
        ),
        ("settings_projects_set", json!({"repos": {}})),
        ("settings_roles_set", json!({"roles": {"human": ["admin"]}})),
        ("settings_github_set", json!({"repo": "o/n", "token": "t"})),
        ("settings_rules_set", json!({"movement": "open"})),
    ] {
        let err = settings
            .call(op, Some(&human), body)
            .expect("op")
            .expect_err(op);
        assert!(
            format!("{err:?}").contains("--settings-dir"),
            "{op}: {err:?}"
        );
    }
    assert!(
        settings
            .call("board_list", Some(&human), json!({}))
            .is_none(),
        "other operations are not its"
    );
}

#[test]
fn an_admin_turns_movement_rules_off_and_on_and_the_choice_is_kept() {
    let f = start("rules");
    let builder = f.add_agent("claude-cloud", "builder");
    let (_, out, _) = f.admin("settings_get", &json!({}));
    assert_eq!(out["rules"], json!({"movement": "enforced"}));
    let (_, card, _) = call(f.addr, &builder, "card_add", &json!({"title": "t"}));
    let id = card["card"]["id"].as_str().expect("id").to_owned();
    let to = |column: &str| json!({"id": id, "to": column});
    // Enforced: a builder may not move into Ready, and Backlog -> Done is no move at all.
    assert_eq!(call(f.addr, &builder, "card_move", &to("ready")).0, 403);
    assert_eq!(call(f.addr, &f.admin, "card_move", &to("done")).0, 422);

    let (status, _, text) = f.admin("settings_rules_set", &json!({"movement": "open"}));
    assert_eq!(status, 200, "{text}");
    assert_eq!(load_rules(&f.file("rules.json")), Ok(MoveRules::Open));
    let (status, moved, text) = call(f.addr, &builder, "card_move", &to("ready"));
    assert_eq!(
        (status, moved["card"]["column"].as_str()),
        (200, Some("ready")),
        "{text}"
    );
    let (status, _, text) = call(f.addr, &builder, "card_move", &to("review"));
    assert_eq!(status, 200, "{text}");

    f.admin("settings_rules_set", &json!({"movement": "enforced"}));
    assert_eq!(load_rules(&f.file("rules.json")), Ok(MoveRules::Enforced));
    assert_eq!(call(f.addr, &builder, "card_move", &to("backlog")).0, 422);
    assert_eq!(
        f.admin("settings_rules_set", &json!({"movement": "sometimes"}))
            .0,
        422,
        "an unknown setting is refused"
    );
}

#[test]
fn everything_survives_a_restart() {
    let f = start("restart");
    f.add_agent("claude", "builder");
    f.admin(
        "settings_projects_set",
        &json!({"repos": {"rusty_mill": {"rl": ["crates/apps/rocket_league"]}}}),
    );
    let (_, out, _) = f.admin("settings_get", &json!({}));
    let mut roles = out["roles"].clone();
    roles["tester"] = json!(["qa"]);
    f.admin("settings_roles_set", &json!({"roles": roles}));

    // What `main` would load next time.
    let roles = load_roles(&f.file("roles.json")).expect("roles");
    let projects = Projects::load(&f.file("projects.json")).expect("projects");
    let creds = Credentials::load(&f.file("identities.json"), &roles, &projects).expect("creds");
    assert!(roles.0.contains_key("tester") && projects.contains("rl") && creds.contains("claude"));
}

/// POST `body` to `path` with no credential (OAuth's own endpoints): the status and JSON.
fn post_open(addr: SocketAddr, path: &str, body: &str) -> (u16, Value) {
    let raw = format!(
        "POST {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    let mut sock = TcpStream::connect(addr).expect("connect");
    sock.write_all(raw.as_bytes()).expect("write");
    let mut out = String::new();
    sock.read_to_string(&mut out).expect("read");
    let status = out
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .expect("status");
    let text = out.split("\r\n\r\n").nth(1).unwrap_or("");
    (status, serde_json::from_str(text).unwrap_or(Value::Null))
}

#[test]
fn settings_describe_each_capability_and_the_server() {
    let f = start("about");
    let (_, out, _) = f.admin("settings_get", &json!({}));
    for cap in out["capabilities"].as_array().expect("caps") {
        let about = out["capability_about"][cap.as_str().expect("name")]
            .as_str()
            .unwrap_or("");
        assert!(!about.is_empty(), "{cap} is described");
    }
    assert_eq!(out["server"]["tick"], Value::Null);
    assert_eq!(out["server"]["issuer"], Value::Null);
    assert_eq!(out["server"]["hosts"], json!(["127.0.0.1"]));
    assert!(
        out["paths"]["roles"]
            .as_str()
            .is_some_and(|p| p.ends_with("roles.json"))
    );
}

#[test]
fn with_oauth_off_there_are_no_apps_to_revoke() {
    let f = start("apps_off");
    let (_, out, _) = f.admin("settings_get", &json!({}));
    assert_eq!(out["apps"], Value::Null);
    let (status, _, text) = f.admin("settings_app_revoke", &json!({"client_id": "x"}));
    assert_eq!(status, 422, "{text}");
    assert!(text.contains("--issuer"), "{text}");
}

#[test]
fn an_admin_lists_and_revokes_signed_in_apps() {
    let f = start_with("apps", true);
    let (status, client) = post_open(
        f.addr,
        "/register",
        r#"{"client_name":"claude.ai","redirect_uris":["https://claude.ai/cb"]}"#,
    );
    assert_eq!(status, 201, "{client}");
    let id = client["client_id"].as_str().expect("id").to_owned();

    let (_, out, _) = f.admin("settings_get", &json!({}));
    assert_eq!(out["server"]["issuer"], ISSUER);
    let apps = out["apps"].as_array().expect("apps");
    assert_eq!(apps.len(), 1);
    assert_eq!(
        (&apps[0]["client_id"], &apps[0]["client_name"]),
        (&json!(id), &json!("claude.ai"))
    );

    let token = f.add_agent("gemini", "builder");
    let (status, _, _) = call(
        f.addr,
        &token,
        "settings_app_revoke",
        &json!({"client_id": id}),
    );
    assert_eq!(status, 403, "revoking needs admin");

    let (status, gone, text) = f.admin("settings_app_revoke", &json!({"client_id": id}));
    assert_eq!(status, 200, "{text}");
    assert_eq!(gone["client_id"], json!(id));
    let (_, out, _) = f.admin("settings_get", &json!({}));
    assert_eq!(out["apps"], json!([]));
    let (status, _, _) = f.admin("settings_app_revoke", &json!({"client_id": id}));
    assert_eq!(status, 422, "an unknown app is refused");
}
