//! What a settings caller may hand out: nothing it does not hold, and `admin` or `enrol` only
//! from a person's own token, never through a shared connector. Every call leaves an audit record.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, RwLock};

use rusty_mill_board::agents::{Grant, Registry};
use rusty_mill_board::api::{CallError, Identity, Shared};
use rusty_mill_board::board::{Board, Roles};
use rusty_mill_board::domain::{BoardError, Capability};
use rusty_mill_board::identities::Credentials;
use rusty_mill_board::projects::Projects;
use rusty_mill_board::settings::{Files, Settings};
use rusty_mill_board::store::{MemStore, Store};
use rusty_mill_board::syncer::Syncer;
use serde_json::{Value, json};

use Capability::{Admin, Build, Enrol, Test, Triage};

struct Rig {
    settings: Settings,
    board: Arc<Shared>,
    dir: std::path::PathBuf,
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn caps(list: &[Capability]) -> BTreeSet<Capability> {
    list.iter().copied().collect()
}

fn rig(name: &str) -> Rig {
    let dir = std::env::temp_dir().join(format!("mill_guards_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("dir");
    let roles = Roles(BTreeMap::from([
        (
            "boss".to_owned(),
            caps(&[Admin, Enrol, Build, Test, Triage]),
        ),
        ("mid".to_owned(), caps(&[Admin, Build])),
        ("dev".to_owned(), caps(&[Build])),
        ("dev2".to_owned(), caps(&[Build, Test])),
    ]));
    let board: Arc<Shared> = Arc::new(
        Board::new(Box::new(MemStore::default()) as Box<dyn Store>, roles)
            .with_projects(Projects::default())
            .with_agents(Registry::default()),
    );
    // The shared connector acts as `boss`, so a reset hands an agent that role back.
    let creds = Credentials::parse(
        r#"[{"token":"conn-token-0123456789","actor":"conn","role":"boss"},
            {"token":"nano-token-0123456789","actor":"nano","role":"mid"}]"#,
        &board.roles(),
        &board.projects(),
    )
    .expect("credentials");
    let settings = Settings::new(
        board.clone(),
        Files::in_dir(&dir),
        Some(Arc::new(RwLock::new(creds))),
        Syncer::new(board.clone()),
        None,
    );
    Rig {
        settings,
        board,
        dir,
    }
}

fn connector(role: &str) -> Identity {
    Identity {
        actor: "conn".into(),
        role: role.into(),
        projects: None,
        connector: true,
        provider: None,
    }
}

fn person(role: &str) -> Identity {
    Identity {
        actor: "nano".into(),
        role: role.into(),
        projects: None,
        connector: false,
        provider: None,
    }
}

impl Rig {
    /// A registered agent on the connector, acting with `role`.
    fn agent(&self, role: &str) -> (String, String) {
        self.board
            .register_agent(
                "conn",
                Grant {
                    role: Some(role.to_owned()),
                    projects: None,
                },
            )
            .expect("register")
    }

    fn call(
        &self,
        who: &Identity,
        agent: Option<&(String, String)>,
        op: &str,
        mut args: Value,
    ) -> Result<Value, CallError> {
        if let Some((id, key)) = agent {
            args["agent_id"] = json!(id);
            args["agent_key"] = json!(key);
        }
        self.settings
            .call(op, Some(who), args)
            .expect("a settings operation")
    }
}

fn refused(r: Result<Value, CallError>, why: &str) {
    match r {
        Err(CallError::Board(BoardError::Refused(m))) => assert!(m.contains(why), "{m}"),
        other => panic!("expected a refusal containing {why:?}, got {other:?}"),
    }
}

#[test]
fn a_connector_agent_grants_only_what_it_holds_and_never_admin_or_enrol() {
    let r = rig("grant");
    let conn = connector("mid");
    let me = r.agent("mid");
    let target = r.agent("dev");
    let set = |role: &str| {
        r.call(
            &conn,
            Some(&me),
            "settings_registered_agent_set",
            json!({"id": target.0, "agent_role": role}),
        )
    };
    // Admin through a connector: refused, and so is a role it does not hold.
    refused(set("boss"), "not through a shared connector");
    refused(set("mid"), "not through a shared connector");
    refused(set("dev2"), "you do not hold Test");
    // What it holds, it may give.
    set("dev").expect("a capability it holds");
    assert_eq!(
        r.board
            .agent_list()
            .iter()
            .find(|a| a.id == target.0)
            .and_then(|a| a.grant.role.clone())
            .as_deref(),
        Some("dev")
    );
}

#[test]
fn a_persons_own_token_may_grant_admin_but_not_beyond_its_other_capabilities() {
    let r = rig("person");
    let target = r.agent("dev");
    let nano = person("mid"); // holds Admin and Build, not Test
    let set = |role: &str| {
        r.call(
            &nano,
            None,
            "settings_registered_agent_set",
            json!({"id": target.0, "agent_role": role}),
        )
    };
    set("mid").expect("admin is a person's to give");
    refused(set("dev2"), "you do not hold Test");
}

#[test]
fn a_connector_agent_cannot_add_admin_to_a_role_nor_a_capability_it_lacks() {
    let r = rig("roles");
    let conn = connector("mid");
    let me = r.agent("mid");
    let roles = |dev: &[Capability]| {
        let mut all = r.board.roles();
        all.0.insert("dev".into(), caps(dev));
        json!({"roles": all})
    };
    refused(
        r.call(
            &conn,
            Some(&me),
            "settings_roles_set",
            roles(&[Build, Admin]),
        ),
        "not through a shared connector",
    );
    refused(
        r.call(
            &conn,
            Some(&me),
            "settings_roles_set",
            roles(&[Build, Test]),
        ),
        "you do not hold Test",
    );
    // Narrowing, or adding what it holds, is fine.
    r.call(&conn, Some(&me), "settings_roles_set", roles(&[Build]))
        .expect("unchanged");
}

#[test]
fn every_settings_call_is_recorded_including_the_refused_and_denied_without_its_arguments() {
    let r = rig("audit");
    let conn = connector("mid");
    let me = r.agent("mid");
    let low = r.agent("dev");
    let target = r.agent("dev");
    let _ = r.call(
        &conn,
        Some(&me),
        "settings_registered_agent_set",
        json!({"id": target.0, "agent_role": "boss"}),
    );
    let _ = r.call(
        &conn,
        Some(&low),
        "settings_registered_agent_set",
        json!({"id": target.0, "agent_role": "dev"}),
    );
    let _ = r.call(
        &conn,
        Some(&me),
        "settings_registered_agent_set",
        json!({"id": target.0, "agent_role": "dev"}),
    );
    let out = r
        .call(&conn, Some(&me), "settings_audit", json!({}))
        .expect("audit");
    let records = out["records"].as_array().expect("records");
    let results: Vec<_> = records
        .iter()
        .map(|x| {
            (
                x["op"].as_str().unwrap_or_default(),
                x["result"].as_str().unwrap_or_default(),
            )
        })
        .collect();
    assert_eq!(
        results,
        [
            ("settings_registered_agent_set", "refused"),
            ("settings_registered_agent_set", "denied"),
            ("settings_registered_agent_set", "ok"),
        ]
    );
    assert_eq!(records[0]["via_connector"], true);
    let text = out.to_string();
    assert!(
        !text.contains(&me.1) && !text.contains("agent_key"),
        "no key in the record: {text}"
    );
    // A read-only board never needs the audit file to exist, and its own read is not a record yet.
    assert!(r.dir.join("settings-audit.jsonl").exists());
}

#[test]
fn a_connector_cannot_reset_an_agent_to_the_connectors_admin_role_or_rotate_a_token() {
    let r = rig("reset");
    let conn = connector("mid");
    let me = r.agent("mid");
    let target = r.agent("dev");
    // A reset would hand `target` the connector's whole role, `boss`, with admin and enrol.
    refused(
        r.call(
            &conn,
            Some(&me),
            "settings_registered_agent_set",
            json!({"id": target.0}),
        ),
        "not through a shared connector",
    );
    // A new raw token is a person's act: a connector may not rotate a person admin's.
    refused(
        r.call(
            &conn,
            Some(&me),
            "settings_agent_rotate",
            json!({"name": "nano"}),
        ),
        "not through a shared connector",
    );
}

#[test]
fn every_settings_tool_is_an_operation_and_none_carries_a_secret() {
    let r = rig("tools");
    let tools = rusty_mill_board::settings::tools();
    assert!(!tools.is_empty());
    for t in &tools {
        assert_eq!(t.schema["type"], "object", "{}", t.name);
        assert!(
            r.settings
                .call(t.name, Some(&person("mid")), json!({}))
                .is_some(),
            "{} is not a settings operation",
            t.name
        );
        assert!(
            !matches!(
                t.name,
                "settings_agent_add" | "settings_agent_rotate" | "settings_github_set"
            ),
            "{} hands out or takes in a secret",
            t.name
        );
    }
}
