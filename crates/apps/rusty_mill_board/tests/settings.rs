//! Settings over a real socket: who may change what, that a change takes effect at once and is
//! written to its file (secrets owner-only), and the refusals that keep the board usable.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::sync::Arc;

use rusty_mcp_server::{HttpConfig, HttpHandler};
use rusty_mill_board::api::Shared;
use rusty_mill_board::board::{Board, Roles};
use rusty_mill_board::domain::{MoveRules, ProjectStatus};
use rusty_mill_board::identities::Credentials;
use rusty_mill_board::mcp;
use rusty_mill_board::projects::Projects;
use rusty_mill_board::settings::{
    Files, ServerInfo, Settings, load_github, load_roles, load_rules, load_statuses, load_workflow,
    open_credentials,
};
use rusty_mill_board::store::{MemStore, Store};
use rusty_mill_board::syncer::Syncer;
use rusty_mill_board::web::{Auth, Web};
use rusty_mill_board::workflow::Workflow;
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
    let mcp = HttpHandler::new(
        mcp::server_with(board.clone(), Arc::default(), Some(settings.clone())).expect("server"),
        config,
    );
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
fn registered_agents_are_listed_and_an_unknown_one_cannot_be_revoked() {
    let f = start("registered");
    let (status, out, text) = f.admin("settings_get", &json!({}));
    assert_eq!(status, 200, "{text}");
    assert_eq!(out["registered_agents"], json!([]));
    let (status, _, _) = f.admin(
        "settings_registered_agent_revoke",
        &json!({"id": "x/abcdef"}),
    );
    assert_eq!(status, 422);
    // Stale revoke: a dry run by default, and the age is bounded.
    let (status, out, text) = f.admin("settings_registered_agent_revoke_stale", &json!({}));
    assert_eq!(status, 200, "{text}");
    assert_eq!(
        (
            out["dry_run"].clone(),
            out["count"].clone(),
            out["older_than_hours"].clone()
        ),
        (json!(true), json!(0), json!(24))
    );
    assert_eq!(out["unsaved"], json!(false));
    for body in [
        json!({"older_than_hours": 1}),
        json!({"older_than_hours": 721}),
        json!({"hours": 24}),
    ] {
        let (status, _, text) = f.admin("settings_registered_agent_revoke_stale", &body);
        assert!(status == 422 || status == 400, "{body}: {status} {text}");
    }
    let (status, out, _) = f.admin(
        "settings_registered_agent_revoke_stale",
        &json!({"revoke": true}),
    );
    assert_eq!((status, out["dry_run"].clone()), (200, json!(false)));
    // Setting a grant: an unknown agent, and a role or project that does not exist, are refused.
    for body in [
        json!({"id": "x#abcdef", "agent_role": "builder"}),
        json!({"id": "x#abcdef", "agent_role": "nope"}),
        json!({"id": "x#abcdef", "agent_projects": ["nope"]}),
        json!({"id": "x#abcdef", "agent_roles": "builder"}),
        json!({"id": "x#abcdef", "nickname": "x".repeat(41)}),
    ] {
        let (status, _, text) = f.admin("settings_registered_agent_set", &body);
        assert!(status == 422 || status == 400, "{body}: {status} {text}");
    }
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
        "settings_project_link",
        "settings_roles_set",
        "settings_role_rename",
        "settings_github_set",
        "settings_github_clear",
        "settings_registered_agent_revoke",
        "settings_registered_agent_revoke_stale",
        "settings_registered_agent_set",
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
fn a_project_status_is_set_live_written_to_its_file_and_gates_new_cards() {
    let f = start("project_status");
    let (status, out, text) = f.admin(
        "settings_project_status_set",
        &json!({"project": "default", "status": "paused"}),
    );
    assert_eq!(status, 200, "{text}");
    assert_eq!(out["project_status"], json!({"default": "paused"}));
    let saved = load_statuses(&f.file("project-status.json")).expect("file");
    assert_eq!(saved.of("default"), ProjectStatus::Paused);
    let (_, listed, _) = f.admin("board_projects", &json!({}));
    assert_eq!(listed["status"], json!({"default": "paused"}));
    let (status, _, text) = f.admin("card_add", &json!({"title": "x"}));
    assert_eq!(status, 422, "a paused project takes no new cards: {text}");

    let (_, out, _) = f.admin(
        "settings_project_status_set",
        &json!({"project": "default", "status": "active"}),
    );
    assert_eq!(
        out["project_status"],
        json!({}),
        "active is the default and not listed"
    );
    assert_eq!(f.admin("card_add", &json!({"title": "x"})).0, 200);
    let (status, _, _) = f.admin(
        "settings_project_status_set",
        &json!({"project": "nowhere", "status": "paused"}),
    );
    assert_eq!(status, 422, "an unknown project is refused");
}

#[test]
fn roles_are_edited_live_and_written_to_their_file() {
    let f = start("roles");
    let (_, out, _) = f.admin("settings_get", &json!({}));
    assert_eq!(
        out["capabilities"],
        json!([
            "triage",
            "plan",
            "architect",
            "build",
            "test",
            "review",
            "merge",
            "close",
            "record",
            "project",
            "admin",
            "enrol"
        ])
    );
    let mut roles = out["roles"].clone();
    // `qa` is the old name of `test`, still accepted from stored and sent roles.
    roles["tester"] = json!(["qa"]);
    let (status, _, text) = f.admin("settings_roles_set", &json!({"roles": roles}));
    assert_eq!(status, 200, "{text}");
    // The new role can be given to an agent straight away, and is on disk.
    let token = f.add_agent("gemini", "tester");
    let (_, me, _) = call(f.addr, &token, "board_whoami", &json!({}));
    assert_eq!(me["capabilities"], json!(["test"]));
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
fn a_role_is_renamed_with_its_agents() {
    let f = start("rename");
    let builder = f.add_agent("claude", "builder");
    let (_, before, _) = call(f.addr, &builder, "board_whoami", &json!({}));
    let (status, out, text) = f.admin(
        "settings_role_rename",
        &json!({"from": "builder", "to": "maker"}),
    );
    assert_eq!(status, 200, "{text}");
    assert!(out["roles"].get("builder").is_none(), "{out}");
    // The agent moved with it, keeps its token and its capabilities, live and on disk.
    let (_, me, _) = call(f.addr, &builder, "board_whoami", &json!({}));
    assert_eq!(me["role"], "maker");
    assert_eq!(me["capabilities"], before["capabilities"]);
    let roles = load_roles(&f.file("roles.json")).expect("file");
    assert!(roles.0.contains_key("maker") && !roles.0.contains_key("builder"));
    let (_, got, _) = f.admin("settings_get", &json!({}));
    assert!(
        got["agents"]
            .as_array()
            .expect("agents")
            .iter()
            .any(|a| a["actor"] == "claude" && a["role"] == "maker"),
        "{got}"
    );
    // The admin's own role renames too, and the admin can still change settings afterwards.
    assert_eq!(
        f.admin(
            "settings_role_rename",
            &json!({"from": "human", "to": "owner"})
        )
        .0,
        200
    );
    assert_eq!(f.admin("settings_get", &json!({})).0, 200);
    // Refused: an unknown role, a taken name, a bad name; nothing changes.
    for (from, to, why) in [
        ("ghost", "spirit", "no role"),
        ("maker", "qa", "already a role"),
        ("maker", "Bad Name", "a-z"),
    ] {
        let (status, _, text) = f.admin("settings_role_rename", &json!({"from": from, "to": to}));
        assert_eq!(
            (status, text.contains(why)),
            (422, true),
            "{from}->{to}: {text}"
        );
    }
    let (_, after, _) = f.admin("settings_get", &json!({}));
    let mut expected = got["roles"].clone();
    let roles = expected.as_object_mut().expect("roles");
    let human = roles.remove("human").expect("human");
    roles.insert("owner".into(), human);
    assert_eq!(after["roles"], expected);
}

/// The files reload as a restart would (no roles file: the defaults): every agent's role
/// exists and an admin remains.
fn restarts(f: &Fixture) -> Roles {
    let path = f.file("roles.json");
    let roles = if path.exists() {
        load_roles(&path).expect("roles file")
    } else {
        Roles::default()
    };
    open_credentials(&f.file("identities.json"), &roles, &Projects::default()).expect("restart");
    roles
}

/// Finish a rename by hand, as a partial one's error says: move the agent, drop the old role.
fn finish_rename(f: &Fixture, agent: &str, from: &str, to: &str) {
    let (status, _, text) = f.admin(
        "settings_agent_update",
        &json!({"agent": {"name": agent, "role": to}}),
    );
    assert_eq!(status, 200, "{text}");
    let (_, out, _) = f.admin("settings_get", &json!({}));
    let mut roles = out["roles"].clone();
    roles.as_object_mut().expect("roles").remove(from);
    let (status, _, text) = f.admin("settings_roles_set", &json!({"roles": roles}));
    assert_eq!(status, 200, "{text}");
}

#[test]
fn a_rename_is_refused_untouched_while_the_saved_credentials_cannot_be_read() {
    let f = start("rename_fail");
    let builder = f.add_agent("claude", "builder");
    let creds = f.file("identities.json");
    let saved = std::fs::read(&creds).expect("credentials");
    std::fs::remove_file(&creds).expect("remove");
    std::fs::create_dir(&creds).expect("block");
    let (status, _, text) = f.admin(
        "settings_role_rename",
        &json!({"from": "builder", "to": "maker"}),
    );
    assert_eq!(
        (status, text.contains("saved credentials")),
        (422, true),
        "{text}"
    );
    std::fs::remove_dir(&creds).expect("unblock");
    std::fs::write(&creds, saved).expect("restore");
    #[cfg(unix)]
    std::fs::set_permissions(&creds, std::os::unix::fs::PermissionsExt::from_mode(0o600))
        .expect("owner-only");
    // Nothing changed, and the same rename then goes through.
    assert!(!restarts(&f).0.contains_key("maker"));
    assert_eq!(
        f.admin(
            "settings_role_rename",
            &json!({"from": "builder", "to": "maker"})
        )
        .0,
        200
    );
    let (_, me, _) = call(f.addr, &builder, "board_whoami", &json!({}));
    assert_eq!(me["role"], "maker");
    assert!(!restarts(&f).0.contains_key("builder"));
}

#[test]
fn a_role_the_saved_credentials_still_use_is_not_dropped() {
    // A credentials save can fail after the file was replaced: then the file holds the new
    // role while the live set still holds the old. Simulate that end state directly.
    let f = start("saved_creds");
    let builder = f.add_agent("claude", "builder");
    let (_, out, _) = f.admin("settings_get", &json!({}));
    let mut roles = out["roles"].clone();
    roles["maker"] = roles["builder"].clone();
    assert_eq!(
        f.admin("settings_roles_set", &json!({"roles": roles})).0,
        200
    );
    let creds = f.file("identities.json");
    let on_disk = std::fs::read_to_string(&creds).expect("credentials");
    let landed = on_disk.replace(r#""role": "builder""#, r#""role": "maker""#);
    assert_ne!(on_disk, landed, "the agent's role is in the file");
    std::fs::write(&creds, landed).expect("simulate a save that landed");
    // Dropping either name is refused: live needs builder, the saved file needs maker.
    for drop in ["maker", "builder"] {
        let mut without = roles.clone();
        without.as_object_mut().expect("roles").remove(drop);
        let (status, _, text) = f.admin("settings_roles_set", &json!({"roles": without}));
        assert_eq!(status, 422, "dropping {drop}: {text}");
    }
    let (_, _, text) = {
        let mut without = roles.clone();
        without.as_object_mut().expect("roles").remove("maker");
        f.admin("settings_roles_set", &json!({"roles": without}))
    };
    assert!(text.contains("saved credentials"), "{text}");
    restarts(&f);
    // Saving the agent makes live and disk agree; then the old name can go.
    finish_rename(&f, "claude", "builder", "maker");
    let (_, me, _) = call(f.addr, &builder, "board_whoami", &json!({}));
    assert_eq!(me["role"], "maker");
    assert!(!restarts(&f).0.contains_key("builder"));
}

#[test]
fn an_agent_is_not_given_a_role_the_saved_roles_file_lacks() {
    // A rename's last roles write can fail after replacing the file: agents hold the new name,
    // the file lacks the old one, the live set still has both. Simulate that end state.
    let f = start("saved_roles");
    let builder = f.add_agent("claude", "builder");
    let (_, out, _) = f.admin("settings_get", &json!({}));
    let mut both = out["roles"].clone();
    both["maker"] = both["builder"].clone();
    assert_eq!(
        f.admin("settings_roles_set", &json!({"roles": both})).0,
        200
    );
    let to_maker = json!({"agent": {"name": "claude", "role": "maker"}});
    assert_eq!(f.admin("settings_agent_update", &to_maker).0, 200);
    let mut landed = both.clone();
    landed.as_object_mut().expect("roles").remove("builder");
    std::fs::write(
        f.file("roles.json"),
        serde_json::to_string_pretty(&landed).expect("json"),
    )
    .expect("simulate a roles write that landed");
    // Giving an agent the role the file lacks is refused (live alone would allow it), and so
    // is a new agent with it; a restart still loads.
    let back = json!({"agent": {"name": "claude", "role": "builder"}});
    let (status, _, text) = f.admin("settings_agent_update", &back);
    assert_eq!(
        (status, text.contains("saved roles file")),
        (422, true),
        "{text}"
    );
    let (status, _, _) = f.admin(
        "settings_agent_add",
        &json!({"agent": {"name": "codex", "role": "builder"}}),
    );
    assert_eq!(status, 422);
    restarts(&f);
    // Recovery either way: re-save the live roles, then the update goes through ...
    assert_eq!(
        f.admin("settings_roles_set", &json!({"roles": both})).0,
        200
    );
    assert_eq!(f.admin("settings_agent_update", &back).0, 200);
    let (_, me, _) = call(f.addr, &builder, "board_whoami", &json!({}));
    assert_eq!(me["role"], "builder");
    restarts(&f);
    // ... or, as the rename's error says, remove the old name.
    assert_eq!(f.admin("settings_agent_update", &to_maker).0, 200);
    assert_eq!(
        f.admin("settings_roles_set", &json!({"roles": landed})).0,
        200
    );
    assert!(!restarts(&f).0.contains_key("builder"));
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
fn a_registry_with_a_spanning_project_is_saved_read_back_and_kept_while_cards_use_it() {
    let f = start("projects");
    let registry = json!({
        "version": 2,
        "repos": {"mill": {"github": "rusty-mill/rusty_mill"}, "py": {}},
        "projects": {"tooling": {"repos": {"mill": [".github"], "py": ["ci"]}, "default_repo": "mill"}},
    });
    let (status, _, text) = f.admin("settings_projects_set", &json!({"registry": registry}));
    assert_eq!(status, 200, "{text}");
    let on_disk = std::fs::read_to_string(f.file("projects.json")).expect("file");
    assert!(on_disk.contains("\"version\": 2") && on_disk.contains("rusty-mill/rusty_mill"));
    let (status, got, text) = f.admin("settings_get", &json!({}));
    assert_eq!(status, 200, "{text}");
    assert_eq!(
        got["registry"]["projects"]["tooling"]["default_repo"],
        "mill"
    );
    assert_eq!(
        got["registry"]["repos"]["mill"]["github"],
        "rusty-mill/rusty_mill"
    );
    // One of the two forms, never both or neither.
    for both in [json!({"registry": registry, "repos": {}}), json!({})] {
        assert_eq!(f.admin("settings_projects_set", &both).0, 422, "{both}");
    }
    // A card in the second repository stops the project from losing it.
    let (status, _, text) = f.admin(
        "card_add",
        &json!({"title": "t", "project": "tooling", "repo": "py"}),
    );
    assert_eq!(status, 200, "{text}");
    let before = std::fs::read_to_string(f.file("projects.json")).expect("read");
    let narrowed = json!({
        "version": 2,
        "repos": {"mill": {}, "py": {}},
        "projects": {"tooling": {"repos": {"mill": [".github"]}}},
    });
    let (status, _, text) = f.admin("settings_projects_set", &json!({"registry": narrowed}));
    assert_eq!(
        (status, text.contains("move the card")),
        (422, true),
        "{text}"
    );
    assert_eq!(
        std::fs::read_to_string(f.file("projects.json")).expect("read"),
        before
    );
}

#[test]
fn a_project_is_linked_to_a_repository_and_unlinked_again() {
    let f = start("link");
    let registry = json!({
        "version": 2,
        "repos": {"mill": {}, "py": {}},
        "projects": {"tooling": {"repos": {"mill": [".github"]}}},
    });
    assert_eq!(
        f.admin("settings_projects_set", &json!({"registry": registry}))
            .0,
        200
    );
    let link = |extra: Value| {
        let mut body = json!({"project": "tooling", "repo": "py"});
        body.as_object_mut()
            .expect("object")
            .extend(extra.as_object().cloned().unwrap_or_default());
        f.admin("settings_project_link", &body)
    };
    let (status, _, text) = link(json!({"paths": ["ci"]}));
    assert_eq!(status, 200, "{text}");
    let on_disk = std::fs::read_to_string(f.file("projects.json")).expect("file");
    assert!(
        on_disk.contains("\"version\": 2") && on_disk.contains("\"ci\""),
        "{on_disk}"
    );
    let (_, got, _) = f.admin("settings_get", &json!({}));
    assert_eq!(
        got["registry"]["projects"]["tooling"]["repos"]["py"],
        json!(["ci"])
    );
    // Refused: an unknown repository; and a repository a card is in.
    let (status, _, _) = f.admin(
        "settings_project_link",
        &json!({"project": "tooling", "repo": "nope"}),
    );
    assert_eq!(status, 422);
    let (status, _, text) = f.admin(
        "card_add",
        &json!({"title": "t", "project": "tooling", "repo": "py"}),
    );
    assert_eq!(status, 200, "{text}");
    let (status, _, text) = link(json!({"unlink": true}));
    assert_eq!(
        (status, text.contains("move the card")),
        (422, true),
        "{text}"
    );
    // The other link can go; the project keeps its last repository.
    let (status, _, text) = f.admin(
        "settings_project_link",
        &json!({"project": "tooling", "repo": "mill", "unlink": true}),
    );
    assert_eq!(status, 200, "{text}");
    let (status, _, text) = f.admin(
        "settings_project_link",
        &json!({"project": "tooling", "repo": "py", "unlink": true}),
    );
    assert_eq!(
        (status, text.contains("only repository")),
        (422, true),
        "{text}"
    );
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
fn auto_close_is_off_unless_set_and_survives_a_save_that_omits_it() {
    let f = start("autoclose");
    let set = |extra: Value| {
        let mut body = json!({"repo": "owner/name", "token": "tok", "api": "http://127.0.0.1:9"});
        body.as_object_mut()
            .expect("object")
            .extend(extra.as_object().expect("object").clone());
        f.admin("settings_github_set", &body)
    };
    let stored = || {
        load_github(&f.file("github.json"))
            .expect("file")
            .expect("some")
            .auto_close
    };
    // Missing means off.
    assert_eq!(set(json!({})).0, 200);
    assert!(!stored());
    // A wrong type is refused and changes nothing.
    assert_eq!(set(json!({"auto_close": "yes"})).0, 422);
    assert!(!stored());
    // On, and kept by a later save that says nothing about it.
    let (status, out, text) = set(json!({"auto_close": true}));
    assert_eq!(
        (status, out["auto_close"].as_bool()),
        (200, Some(true)),
        "{text}"
    );
    assert!(stored());
    assert_eq!(set(json!({"interval_secs": 60})).0, 200);
    assert!(stored());
    assert_eq!(set(json!({"auto_close": false})).0, 200);
    assert!(!stored());
}

#[test]
fn without_files_settings_can_be_read_but_not_changed() {
    let board: Arc<Shared> = Arc::new(Board::new(Box::new(MemStore::default()), Roles::default()));
    let settings = Settings::read_only(board);
    let human = rusty_mill_board::api::Identity {
        actor: "nano".into(),
        role: "human".into(),
        projects: None,
        connector: false,
    };
    let get = settings
        .call("settings_get", Some(&human), json!({}))
        .expect("op")
        .expect("ok");
    assert_eq!(
        get["files"],
        json!({"agents": false, "projects": false, "roles": false, "github": false, "rules": false, "workflow": false, "project_status": false})
    );
    for (op, body) in [
        (
            "settings_agent_add",
            json!({"agent": {"name": "x", "role": "builder"}}),
        ),
        ("settings_projects_set", json!({"repos": {}})),
        (
            "settings_project_status_set",
            json!({"project": "default", "status": "paused"}),
        ),
        ("settings_roles_set", json!({"roles": {"human": ["admin"]}})),
        ("settings_github_set", json!({"repo": "o/n", "token": "t"})),
        ("settings_rules_set", json!({"movement": "open"})),
        (
            "settings_workflow_set",
            json!({"workflow": Workflow::default()}),
        ),
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
fn an_admin_adds_renames_reorders_and_removes_columns() {
    let f = start("columns");
    let builder = f.add_agent("claude-cloud", "builder");
    let (_, out, _) = f.admin("settings_get", &json!({}));
    let mut flow = out["workflow"].clone();
    assert_eq!(flow, json!(Workflow::default()));

    // Add a queue column "Design" after Backlog, relabel Ready, and route Backlog through it.
    let columns = flow["columns"].as_array_mut().expect("columns");
    columns.insert(
        1,
        json!({"id": "design", "label": "Design", "kind": "work"}),
    );
    columns[2]["label"] = json!("Approved");
    flow["transitions"]
        .as_array_mut()
        .expect("transitions")
        .push(json!({"from": "backlog", "to": "design", "cap": "build"}));
    let (status, set, text) = f.admin("settings_workflow_set", &json!({"workflow": flow}));
    assert_eq!(status, 200, "{text}");
    assert_eq!(set["workflow"]["columns"][2]["label"], "Approved");
    assert_eq!(
        load_workflow(&f.file("workflow.json")).map(|w| json!(w)),
        Ok(set["workflow"].clone())
    );
    let (_, listed, _) = call(f.addr, &builder, "board_columns", &json!({}));
    assert_eq!(listed["columns"][1]["id"], "design");

    // Cards move through it, under the capability the admin gave the move.
    let (_, card, _) = call(f.addr, &builder, "card_add", &json!({"title": "t"}));
    let id = card["card"]["id"].as_str().expect("id").to_owned();
    let (status, moved, text) = call(
        f.addr,
        &builder,
        "card_move",
        &json!({"id": id, "to": "design"}),
    );
    assert_eq!(
        (status, moved["card"]["column"].as_str()),
        (200, Some("design")),
        "{text}"
    );

    // A column holding cards cannot be removed; once empty it can.
    let mut without = set["workflow"].clone();
    without["columns"]
        .as_array_mut()
        .expect("columns")
        .remove(1);
    without["transitions"]
        .as_array_mut()
        .expect("transitions")
        .retain(|t| t["to"] != "design");
    let (status, _, text) = f.admin("settings_workflow_set", &json!({"workflow": without}));
    assert_eq!(status, 422, "{text}");
    assert!(text.contains("design still has cards"), "{text}");
    assert_eq!(
        load_workflow(&f.file("workflow.json")).map(|w| json!(w)),
        Ok(set["workflow"].clone())
    );
    call(
        f.addr,
        &f.admin,
        "card_move",
        &json!({"id": id, "to": "blocked"}),
    );
    let (status, _, text) = f.admin("settings_workflow_set", &json!({"workflow": without}));
    assert_eq!(status, 200, "{text}");

    // The anchors stay, and an inconsistent workflow is refused with the reason.
    let mut no_done = without.clone();
    no_done["columns"]
        .as_array_mut()
        .expect("columns")
        .retain(|c| c["id"] != "done");
    let (status, _, text) = f.admin("settings_workflow_set", &json!({"workflow": no_done}));
    assert_eq!(status, 422);
    assert!(text.contains("done cannot be removed"), "{text}");
    let (status, _, _) = call(
        f.addr,
        &builder,
        "settings_workflow_set",
        &json!({"workflow": without}),
    );
    assert_eq!(status, 403, "only an admin");
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

#[test]
fn a_malformed_project_status_file_is_refused_at_load() {
    let dir = std::env::temp_dir().join(format!("mb-status-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("dir");
    let path = dir.join("project-status.json");
    for bad in ["not json", r#"{"default": "sleeping"}"#] {
        std::fs::write(&path, bad).expect("write");
        assert!(load_statuses(&path).is_err(), "{bad}");
    }
    std::fs::remove_dir_all(&dir).expect("clean up");
}

/// A lead role that may enrol, plus one worker role, saved through settings.
fn with_lead_role(f: &Fixture) {
    let (_, out, _) = f.admin("settings_get", &json!({}));
    let mut roles = out["roles"].clone();
    roles["lead"] = json!(["build", "plan", "test", "enrol"]);
    roles["worker"] = json!(["build"]);
    let (status, _, text) = f.admin("settings_roles_set", &json!({"roles": roles}));
    assert_eq!(status, 200, "{text}");
}

fn lead_with(f: &Fixture, name: &str, projects: Option<Value>) -> String {
    let (status, out, text) = f.admin(
        "settings_agent_add",
        &json!({"agent": {"name": name, "role": "lead", "projects": projects}}),
    );
    assert_eq!(status, 200, "{text}");
    out["token"].as_str().expect("token").to_owned()
}

#[test]
fn a_lead_mints_a_worker_that_works_at_once_within_its_ceilings() {
    let f = start("enrol");
    with_lead_role(&f);
    let lead = lead_with(&f, "chief", None);
    let (status, out, text) = call(
        f.addr,
        &lead,
        "agent_enrol",
        &json!({"label": "w1", "agent_role": "worker"}),
    );
    assert_eq!(status, 200, "{text}");
    assert_eq!(out["agent"], "chief.w1");
    let worker = out["token"].as_str().expect("token");
    let (_, me, _) = call(f.addr, worker, "board_whoami", &json!({}));
    assert_eq!(
        (me["actor"].clone(), me["role"].clone()),
        (json!("chief.w1"), json!("worker"))
    );
    // The admin sees it under its lead, and removing the lead removes it.
    let (_, got, _) = f.admin("settings_get", &json!({}));
    let agents = got["agents"].as_array().expect("agents");
    assert!(
        agents
            .iter()
            .any(|a| a["actor"] == "chief.w1" && a["parent"] == "chief")
    );
    assert_eq!(
        f.admin("settings_agent_remove", &json!({"name": "chief"}))
            .0,
        200
    );
    let (status, ..) = call(f.addr, worker, "board_whoami", &json!({}));
    assert_eq!(
        status, 401,
        "a removed lead takes its minted agents with it"
    );
}

#[test]
fn enrol_needs_the_capability_and_refuses_anything_wider() {
    let f = start("enrol_ceilings");
    with_lead_role(&f);
    let plain = f.add_agent("plain", "builder");
    let mint = |token: &str, body: Value| call(f.addr, token, "agent_enrol", &body);
    let (status, ..) = mint(&plain, json!({"label": "x", "agent_role": "builder"}));
    assert_eq!(status, 403, "without enrol nothing is minted");

    let lead = lead_with(&f, "chief", None);
    for (role, why) in [
        ("lead", "enrol"),
        ("human", "enrol"),
        ("reviewer", "Review"),
        ("nope", "no role"),
    ] {
        let (status, _, text) = mint(&lead, json!({"label": "x", "agent_role": role}));
        assert_eq!(status, 422, "{role}: {text}");
        assert!(text.contains(why), "{role}: {text}");
    }
    let (status, ..) = mint(&lead, json!({"label": "bad label", "agent_role": "worker"}));
    assert_eq!(status, 422);
}

#[test]
fn a_project_limited_lead_mints_only_inside_its_projects_and_at_most_eight() {
    let f = start("enrol_projects");
    with_lead_role(&f);
    let repos = json!({"rusty_mill": {"alpha": ["crates/alpha"], "beta": ["crates/beta"]}});
    assert_eq!(
        f.admin("settings_projects_set", &json!({"repos": repos})).0,
        200
    );
    let lead = lead_with(&f, "chief", Some(json!(["alpha"])));
    let mint = |body: Value| call(f.addr, &lead, "agent_enrol", &body);
    let (status, _, text) = mint(json!({"label": "a", "agent_role": "worker"}));
    assert_eq!(status, 422, "a limited lead must name projects: {text}");
    let (status, ..) =
        mint(json!({"label": "a", "agent_role": "worker", "agent_projects": ["beta"]}));
    assert_eq!(status, 422, "beta is outside its projects");
    for n in 0..8 {
        let (status, _, text) = mint(
            json!({"label": format!("w{n}"), "agent_role": "worker", "agent_projects": ["alpha"]}),
        );
        assert_eq!(status, 200, "{n}: {text}");
    }
    let (status, _, text) =
        mint(json!({"label": "w8", "agent_role": "worker", "agent_projects": ["alpha"]}));
    assert_eq!(status, 422, "the ninth is refused: {text}");
}

#[test]
fn a_lead_revokes_its_own_agents_and_no_one_elses() {
    let f = start("enrol_revoke");
    with_lead_role(&f);
    let chief = lead_with(&f, "chief", None);
    let other = lead_with(&f, "other", None);
    let mint = |token: &str, label: &str| {
        call(
            f.addr,
            token,
            "agent_enrol",
            &json!({"label": label, "agent_role": "worker"}),
        )
    };
    let worker = mint(&chief, "w").1["token"]
        .as_str()
        .expect("token")
        .to_owned();
    let revoke = |token: &str, label: &str| {
        call(
            f.addr,
            token,
            "agent_enrol_revoke",
            &json!({"label": label}),
        )
        .0
    };
    assert_eq!(revoke(&other, "w"), 422, "another lead cannot revoke it");
    assert_eq!(revoke(&chief, "w"), 200);
    assert_eq!(call(f.addr, &worker, "board_whoami", &json!({})).0, 401);
    assert_eq!(revoke(&chief, "w"), 422, "already gone");
    // A hand-added agent is not its lead's to revoke.
    f.add_agent("chief.hand", "worker");
    assert_eq!(revoke(&chief, "hand"), 422);
}

#[test]
fn minted_agents_survive_a_restart_and_keep_their_lead() {
    let f = start("enrol_restart");
    with_lead_role(&f);
    let lead = lead_with(&f, "chief", None);
    let (status, ..) = call(
        f.addr,
        &lead,
        "agent_enrol",
        &json!({"label": "w", "agent_role": "worker"}),
    );
    assert_eq!(status, 200);
    let roles = restarts(&f);
    let creds = Credentials::load(&f.file("identities.json"), &roles, &Projects::default())
        .expect("reloads");
    assert_eq!(creds.parent_of("chief.w"), Some("chief"));
}

#[test]
fn enrol_refuses_a_misspelt_argument_and_a_name_that_is_taken() {
    let f = start("enrol_args");
    with_lead_role(&f);
    let lead = lead_with(&f, "chief", None);
    let mint = |body: Value| call(f.addr, &lead, "agent_enrol", &body);
    // `projects` where `agent_projects` belongs would otherwise mint an agent for every project.
    let (status, ..) = mint(json!({"label": "w", "agent_role": "worker", "projects": ["x"]}));
    assert_eq!(status, 422);
    // Stating your own actor and role is still fine.
    let (status, ..) =
        mint(json!({"label": "w", "agent_role": "worker", "actor": "chief", "role": "lead"}));
    assert_eq!(status, 200);
    let (status, _, text) = mint(json!({"label": "w", "agent_role": "worker"}));
    assert_eq!((status, text.contains("already")), (422, true), "{text}");
}

#[test]
fn a_credentials_file_with_a_bad_parent_is_refused() {
    let roles = Roles::default();
    let projects = Projects::default();
    let file = |entries: &str| format!("[{entries}]");
    let lead = r#"{"token":"0123456789abcdef0123","actor":"chief","role":"builder"}"#;
    let kid = |actor: &str, parent: &str| {
        format!(
            r#"{{"token":"{actor}-token-0123456789","actor":"{actor}","role":"builder","parent":"{parent}"}}"#
        )
    };
    // A child whose lead is missing, whose name lacks the `lead.` prefix, and a grandchild.
    for bad in [
        file(&format!("{lead},{}", kid("chief.a", "ghost"))),
        file(&format!("{lead},{}", kid("other", "chief"))),
        file(&format!(
            "{lead},{},{}",
            kid("chief.a", "chief"),
            kid("chief.a.b", "chief.a")
        )),
    ] {
        assert!(
            Credentials::parse(&bad, &roles, &projects).is_err(),
            "{bad}"
        );
    }
    let good = file(&format!("{lead},{}", kid("chief.a", "chief")));
    let creds = Credentials::parse(&good, &roles, &projects).expect("good");
    assert_eq!(creds.parent_of("chief.a"), Some("chief"));
}

#[test]
fn narrowing_a_lead_removes_the_agents_it_minted_that_no_longer_fit() {
    let f = start("enrol_prune");
    with_lead_role(&f);
    let (_, got, _) = f.admin("settings_get", &json!({}));
    let mut roles = got["roles"].clone();
    roles["helper"] = json!(["plan"]);
    assert_eq!(
        f.admin("settings_roles_set", &json!({"roles": roles})).0,
        200
    );
    let lead = lead_with(&f, "chief", None);
    let mint = |label: &str, role: &str| {
        let (status, out, text) = call(
            f.addr,
            &lead,
            "agent_enrol",
            &json!({"label": label, "agent_role": role}),
        );
        assert_eq!(status, 200, "{text}");
        out["token"].as_str().expect("token").to_owned()
    };
    let (worker, helper) = (mint("w", "worker"), mint("h", "helper"));
    // Moving the lead to a role without `build` drops the worker but keeps the helper.
    let (status, out, text) = f.admin(
        "settings_agent_update",
        &json!({"agent": {"name": "chief", "role": "helper"}}),
    );
    assert_eq!(status, 200, "{text}");
    assert_eq!(out["removed"], json!(["chief.w"]));
    assert_eq!(call(f.addr, &worker, "board_whoami", &json!({})).0, 401);
    assert_eq!(call(f.addr, &helper, "board_whoami", &json!({})).0, 200);
}

/// One JSON-RPC call to `/mcp`; the reply's `result`.
fn mcp(addr: SocketAddr, token: &str, rpc: &Value) -> Value {
    let body = rpc.to_string();
    let raw = format!(
        "POST /mcp HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\nAuthorization: Bearer {token}\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    let mut sock = TcpStream::connect(addr).expect("connect");
    sock.write_all(raw.as_bytes()).expect("write");
    let mut out = String::new();
    sock.read_to_string(&mut out).expect("read");
    let text = out.split("\r\n\r\n").nth(1).unwrap_or("");
    let start = text.find('{').expect("a json reply");
    let line = text[start..].lines().next().unwrap_or("");
    serde_json::from_str::<Value>(line).expect("json-rpc")["result"].clone()
}

#[test]
fn settings_are_mcp_tools_except_those_that_carry_a_secret() {
    let f = start("mcp_tools");
    let listed = mcp(
        f.addr,
        &f.admin,
        &json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}),
    );
    let names: Vec<&str> = listed["tools"]
        .as_array()
        .expect("tools")
        .iter()
        .filter_map(|t| t["name"].as_str())
        .collect();
    for want in [
        "settings_get",
        "settings_audit",
        "settings_registered_agent_set",
        "settings_projects_set",
    ] {
        assert!(names.contains(&want), "{want} is a tool");
    }
    for secret in [
        "settings_agent_add",
        "settings_agent_rotate",
        "settings_github_set",
        "agent_enrol",
    ] {
        assert!(!names.contains(&secret), "{secret} stays HTTP-only");
    }
    let got = mcp(
        f.addr,
        &f.admin,
        &json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"settings_get","arguments":{}}}),
    );
    let text = got["content"][0]["text"].as_str().expect("text result");
    let parsed: serde_json::Value = serde_json::from_str(text).expect("text is the JSON result");
    assert!(parsed["roles"].is_object(), "{got}");
}

#[test]
fn a_refused_card_write_is_recorded_with_its_reason_and_never_its_token() {
    let f = start("refusals");
    let (_, made, text) = f.admin("card_add", &json!({"title": "needs evidence"}));
    let card = made["card"]["id"].as_str().expect(&text).to_owned();
    let token = f.add_agent("watcher", "reviewer");
    let link = json!({"id": card, "links": ["https://github.com/o/r/pull/1"]});
    let (status, _, text) = call(f.addr, &token, "card_evidence", &link);
    assert_eq!(
        status, 403,
        "a reviewer holds none of close, merge or triage: {text}"
    );
    // A read that fails is not a write the owner has to chase.
    let _ = call(f.addr, &token, "card_get", &json!({"id": "#99999"}));

    let (status, out, text) = f.admin("settings_audit", &json!({"limit": 50}));
    assert_eq!(status, 200, "{text}");
    let seen: Vec<&Value> = out["records"]
        .as_array()
        .expect("records")
        .iter()
        .filter(|r| r["op"] == "card_evidence")
        .collect();
    assert_eq!(seen.len(), 1, "{text}");
    assert_eq!(seen[0]["result"], "denied");
    assert_eq!(seen[0]["actor"], "watcher");
    assert_eq!(seen[0]["card"], json!(card));
    assert!(
        seen[0]["why"].as_str().is_some_and(|w| w.contains("Close")),
        "{text}"
    );
    assert!(
        !text.contains(&token) && !text.contains("pull/1"),
        "no secret or argument: {text}"
    );
    assert!(
        out["records"]
            .as_array()
            .expect("records")
            .iter()
            .all(|r| r["op"] != "card_get")
    );
}

#[test]
fn a_refusal_is_attributed_to_the_credential_and_the_claim_is_kept_apart() {
    let f = start("refusal_actor");
    let token = f.add_agent("watcher", "reviewer");
    let (_, made, text) = f.admin("card_add", &json!({"title": "t"}));
    let card = made["card"]["id"].as_str().expect(&text).to_owned();
    // The credential fixes the actor, so naming another is refused: and recorded as a claim only.
    let (status, _, _) = call(
        f.addr,
        &token,
        "card_comment",
        &json!({"id": card, "text": "hi", "actor": "nano"}),
    );
    assert_eq!(status, 422);
    let (_, out, text) = f.admin("settings_audit", &json!({"limit": 50}));
    let rec = out["records"]
        .as_array()
        .expect("records")
        .iter()
        .find(|r| r["op"] == "card_comment")
        .unwrap_or_else(|| panic!("{text}"));
    assert_eq!(
        rec["actor"], "watcher",
        "the verified credential, not the claim"
    );
    assert_eq!(rec["claimed_actor"], "nano");
}

#[test]
fn a_malformed_argument_never_reaches_the_record_through_its_reason() {
    let f = start("refusal_args");
    let token = f.add_agent("dev", "builder");
    let marker = "SECRET-MARKER-9f3a";
    // A string where a list belongs: serde's own message would quote it.
    let (status, _, text) = call(
        f.addr,
        &token,
        "card_add",
        &json!({"title": "t", "tags": marker}),
    );
    assert_eq!(status, 400, "{text}");
    let (_, out, text) = f.admin("settings_audit", &json!({"limit": 50}));
    assert!(!text.contains(marker), "{text}");
    let rec = out["records"]
        .as_array()
        .expect("records")
        .iter()
        .find(|r| r["op"] == "card_add")
        .unwrap_or_else(|| panic!("{text}"));
    assert_eq!(rec["result"], "bad_args");
    assert_eq!(rec["why"], "the arguments did not parse");
}

#[test]
fn refusal_records_stop_at_the_size_cap() {
    let f = start("refusal_cap");
    let token = f.add_agent("watcher", "reviewer");
    let path = f.file("settings-audit.jsonl");
    let junk = "x".repeat(1023) + "\n";
    let mut file = std::fs::File::create(&path).expect("file");
    for _ in 0..rusty_mill_board::settings::MAX_REFUSAL_FILE / 1024 {
        file.write_all(junk.as_bytes()).expect("fill");
    }
    drop(file);
    let before = std::fs::metadata(&path).expect("meta").len();
    let link = json!({"id": "#1", "links": ["https://github.com/o/r/pull/1"]});
    let (status, _, _) = call(f.addr, &token, "card_evidence", &link);
    assert!(status >= 400);
    assert_eq!(
        std::fs::metadata(&path).expect("meta").len(),
        before,
        "nothing appended past the cap"
    );
}

fn history(f: &Fixture, body: &Value) -> Vec<Value> {
    let (status, out, text) = f.admin("settings_history", body);
    assert_eq!(status, 200, "{text}");
    out["entries"].as_array().expect(&text).clone()
}

#[test]
fn a_settings_change_is_recorded_with_its_old_and_new_value() {
    let f = start("history_rules");
    assert!(history(&f, &json!({})).is_empty());
    let (status, _, text) = f.admin("settings_rules_set", &json!({"movement": "open"}));
    assert_eq!(status, 200, "{text}");
    let h = history(&f, &json!({}));
    assert_eq!(h.len(), 1, "{h:?}");
    assert_eq!(h[0]["actor"], "admin");
    assert_eq!(h[0]["op"], "settings_rules_set");
    assert_eq!(h[0]["section"], "rules");
    assert_ne!(h[0]["old"], h[0]["new"]);
    assert!(h[0]["at_ms"].as_i64().is_some_and(|t| t > 0));
}

#[test]
fn a_refused_or_unchanged_write_leaves_no_history() {
    let f = start("history_none");
    f.admin("settings_rules_set", &json!({"movement": "open"}));
    // The same value again changes nothing.
    f.admin("settings_rules_set", &json!({"movement": "open"}));
    // A bad role name is refused.
    let (status, _, _) = f.admin("settings_roles_set", &json!({"roles": {}}));
    assert_ne!(status, 200);
    assert_eq!(history(&f, &json!({})).len(), 1);
}

#[test]
fn history_filters_by_section_and_never_holds_a_secret() {
    let f = start("history_secrets");
    let token = f.add_agent("watcher", "reviewer");
    let (status, _, text) = f.admin(
        "settings_github_set",
        &json!({"repo": "o/r", "api": "https://api.github.com", "interval_secs": 60, "token": "ghp_SECRET-MARKER"}),
    );
    assert_eq!(status, 200, "{text}");
    f.admin("settings_rules_set", &json!({"movement": "open"}));

    let (_, _, all) = f.admin("settings_history", &json!({}));
    assert!(
        !all.contains(&token) && !all.contains("SECRET-MARKER"),
        "{all}"
    );
    let agents = history(&f, &json!({"section": "agents"}));
    assert_eq!(agents.len(), 1, "{agents:?}");
    assert!(agents[0]["new"].to_string().contains("watcher"));
    let github = history(&f, &json!({"section": "github"}));
    assert_eq!(github[0]["new"]["has_token"], true);
    assert_eq!(history(&f, &json!({"limit": 1})).len(), 1);
}

#[test]
fn history_needs_admin() {
    let f = start("history_admin");
    let token = f.add_agent("watcher", "reviewer");
    let (status, _, _) = call(f.addr, &token, "settings_history", &json!({}));
    assert_eq!(status, 403);
}
