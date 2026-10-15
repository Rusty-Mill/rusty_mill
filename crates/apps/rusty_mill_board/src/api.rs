//! The operation table. MCP tools and the web API both dispatch through it,
//! so the two surfaces cannot drift: one name, one argument type, one schema.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::board::Board;
use crate::domain::{Actor, BoardError, Card, Column, DEFAULT_PROJECT};
use crate::store::Store;

/// The board every surface shares.
pub type Shared = Board<Box<dyn Store>>;

/// Who a credential belongs to. Set by the transport after it checks a bearer
/// token; when present it fixes the caller's actor and role.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identity {
    /// Stable name, recorded in claims and history.
    pub actor: String,
    /// Role name; see `board_roles`.
    pub role: String,
    /// Projects this credential may work in; `None` is all of them.
    #[serde(default)]
    pub projects: Option<Vec<String>>,
}

/// Why a call failed.
#[derive(Debug)]
pub enum CallError {
    /// The arguments did not parse.
    BadArgs(String),
    /// The board refused or failed.
    Board(BoardError),
}

impl From<BoardError> for CallError {
    fn from(e: BoardError) -> Self {
        Self::Board(e)
    }
}

type RunFn =
    Box<dyn Fn(&Shared, Option<&Identity>, Value) -> Result<Value, CallError> + Send + Sync>;

/// One named operation.
pub struct Op {
    /// Tool name and web path segment.
    pub name: &'static str,
    /// Shown to agents.
    pub description: &'static str,
    /// JSON Schema of the arguments.
    pub schema: Value,
    run: RunFn,
}

impl Op {
    /// Run with raw JSON arguments, as `identity` if the transport
    /// authenticated one.
    ///
    /// # Errors
    /// [`CallError::BadArgs`] if they don't parse, else whatever the board says.
    pub fn call(
        &self,
        board: &Shared,
        identity: Option<&Identity>,
        args: Value,
    ) -> Result<Value, CallError> {
        (self.run)(board, identity, args)
    }
}

fn op<A>(
    name: &'static str,
    description: &'static str,
    f: fn(&Shared, Option<&Identity>, A) -> Result<Value, BoardError>,
) -> Op
where
    A: serde::de::DeserializeOwned + JsonSchema + 'static,
{
    let mut schema = serde_json::to_value(schemars::schema_for!(A)).unwrap_or(Value::Null);
    if let Some(object) = schema.as_object_mut() {
        object.remove("$schema");
        object.remove("title");
    }
    Op {
        name,
        description,
        schema,
        run: Box::new(move |board, identity, args| {
            let args =
                serde_json::from_value(args).map_err(|e| CallError::BadArgs(e.to_string()))?;
            f(board, identity, args).map_err(CallError::Board)
        }),
    }
}

/// Who is acting. With a per-model credential these are fixed by it: omit
/// them (or repeat them exactly). Without one the board trusts what the
/// caller says: a workflow guard between cooperating agents.
#[derive(Deserialize, JsonSchema)]
struct Who {
    /// Stable name of the actor, recorded in claims and history.
    actor: Option<String>,
    /// Role name; see `board_roles`.
    role: Option<String>,
}

/// The acting identity: the credential's if there is one (a disagreeing
/// `actor`/`role` is refused), else the arguments'.
fn acting(b: &Shared, identity: Option<&Identity>, who: &Who) -> Result<Actor, BoardError> {
    match identity {
        Some(i) => {
            let differs = who.actor.as_deref().is_some_and(|a| a != i.actor)
                || who.role.as_deref().is_some_and(|r| r != i.role);
            if differs {
                return Err(BoardError::Invalid(
                    "actor and role are fixed by your credential; omit them",
                ));
            }
            let mut actor = b.roles().actor(&i.actor, &i.role)?;
            actor.projects = i.projects.as_ref().map(|p| p.iter().cloned().collect());
            Ok(actor)
        }
        None => {
            let actor = who
                .actor
                .as_deref()
                .ok_or(BoardError::Invalid("actor is required"))?;
            let role = who
                .role
                .as_deref()
                .ok_or(BoardError::Invalid("role is required"))?;
            b.roles().actor(actor, role)
        }
    }
}

#[derive(Deserialize, JsonSchema)]
struct Nothing {}

#[derive(Deserialize, JsonSchema)]
struct TakeArgs {
    #[serde(flatten)]
    who: Who,
    /// Take this card instead of the next one for your role.
    id: Option<String>,
    /// Only cards of this project (see `board_projects`).
    project: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
struct ListArgs {
    /// Only this column.
    column: Option<Column>,
    /// Only this project (see `board_projects`).
    project: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
struct AssignArgs {
    #[serde(flatten)]
    who: Who,
    /// Card id.
    id: String,
    /// The project to move it to (see `board_projects`).
    project: String,
}

#[derive(Deserialize, JsonSchema)]
struct GetArgs {
    /// Card id.
    id: String,
}

#[derive(Deserialize, JsonSchema)]
struct AddArgs {
    #[serde(flatten)]
    who: Who,
    /// Short title.
    title: String,
    /// Description (markdown).
    body: Option<String>,
    /// Labels.
    tags: Option<Vec<String>>,
    /// The project the card belongs to (default: `default`; see `board_projects`).
    project: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
struct MoveArgs {
    #[serde(flatten)]
    who: Who,
    /// Card id.
    id: String,
    /// Destination column.
    to: Column,
}

#[derive(Deserialize, JsonSchema)]
struct IdArgs {
    #[serde(flatten)]
    who: Who,
    /// Card id.
    id: String,
}

#[derive(Deserialize, JsonSchema)]
struct UpdateArgs {
    #[serde(flatten)]
    who: Who,
    /// Card id.
    id: String,
    /// New title.
    title: Option<String>,
    /// New description.
    body: Option<String>,
    /// New labels (replaces all).
    tags: Option<Vec<String>>,
    /// The card revision you last saw. If the card has moved on, the update is refused (409)
    /// so you can look at what changed instead of overwriting it.
    rev: Option<u64>,
}

#[derive(Deserialize, JsonSchema)]
struct CommentArgs {
    #[serde(flatten)]
    who: Who,
    /// Card id.
    id: String,
    /// Comment text.
    text: String,
}

#[derive(Deserialize, JsonSchema)]
struct EventsArgs {
    /// Return events after this event id (omit for the beginning).
    after: Option<String>,
    /// At most this many (default 100).
    limit: Option<usize>,
    /// Return no events, only the `cursor` at the end of the log right now. Start
    /// a watcher with this, then read the board: anything that changes in between
    /// is delivered by the next poll.
    tail: Option<bool>,
}

fn id(raw: &str) -> Result<Uuid, BoardError> {
    Uuid::parse_str(raw).map_err(|_| BoardError::Invalid("id is not a UUID"))
}

/// The projects a credential is limited to; `None` when unrestricted or unauthenticated.
fn scope(identity: Option<&Identity>) -> Option<&[String]> {
    identity.and_then(|i| i.projects.as_deref())
}

fn in_scope(scope: Option<&[String]>, project: &str) -> bool {
    scope.is_none_or(|s| s.iter().any(|p| p == project))
}

/// `project` as a known project the caller may see, or an error.
fn checked_project(
    b: &Shared,
    scope: Option<&[String]>,
    project: Option<&str>,
) -> Result<(), BoardError> {
    let Some(p) = project else { return Ok(()) };
    if !b.projects().contains(p) {
        return Err(BoardError::Invalid("unknown project"));
    }
    if !in_scope(scope, p) {
        return Err(BoardError::OutOfScope(p.to_owned()));
    }
    Ok(())
}

fn card(c: Card) -> Result<Value, BoardError> {
    serde_json::to_value(json!({ "card": c })).map_err(|e| BoardError::Storage(e.to_string()))
}

fn to_json(v: impl serde::Serialize) -> Result<Value, BoardError> {
    serde_json::to_value(v).map_err(|e| BoardError::Storage(e.to_string()))
}

/// Every operation, in display order.
pub fn ops() -> Vec<Op> {
    vec![
        op(
            "board_list",
            "List cards, optionally one column and/or one project. Columns: backlog, ready, building, qa, review, done, blocked. A credential limited to some projects only sees those.",
            |b, i, a: ListArgs| {
                checked_project(b, scope(i), a.project.as_deref())?;
                let mut cards = b.list(a.column)?;
                cards.retain(|c| {
                    in_scope(scope(i), &c.project)
                        && a.project.as_deref().is_none_or(|p| c.project == p)
                });
                to_json(json!({ "cards": cards }))
            },
        ),
        op(
            "board_projects",
            "List the repositories this board serves, their projects (applications) and the repository paths each project may change: {repos: {repo: {project: [paths]}}}. Cards belong to one project; a repository's own project (named like it) owns every path no other project of that repository does.",
            |b, i, _: Nothing| {
                // A limited credential is told only about the projects it may work in.
                let mut repos = serde_json::to_value(b.projects())
                    .map_err(|e| BoardError::Storage(e.to_string()))?;
                if let (Some(scope), Some(map)) = (scope(i), repos.as_object_mut()) {
                    map.values_mut().for_each(|projects| {
                        if let Some(p) = projects.as_object_mut() {
                            p.retain(|name, _| scope.iter().any(|s| s == name));
                        }
                    });
                    map.retain(|_, projects| projects.as_object().is_some_and(|p| !p.is_empty()));
                }
                to_json(json!({ "repos": repos, "default": DEFAULT_PROJECT }))
            },
        ),
        op(
            "board_roles",
            "List roles and the capabilities each holds.",
            |b, _, _: Nothing| to_json(json!({ "roles": b.roles().0 })),
        ),
        op(
            "board_events",
            "Poll the change log, oldest first. Send the `cursor` of your last reply as `after` to get only newer events (the reply's `cursor` is the last event returned, or your `after` if none). `tail: true` returns just the current end of the log, to start a watcher.",
            |b, _, a: EventsArgs| {
                if a.tail == Some(true) {
                    return to_json(json!({ "events": [], "cursor": b.cursor()? }));
                }
                let after = a.after.as_deref().map(id).transpose()?;
                let events = b.events(after, a.limit.unwrap_or(100))?;
                let cursor = events.last().map(|e| e.id).or(after);
                to_json(json!({ "events": events, "cursor": cursor }))
            },
        ),
        op(
            "board_whoami",
            "Who you are on this board: your actor name, role and what that role may do.",
            |b, i, _: Nothing| {
                let Some(i) = i else {
                    return to_json(json!({
                        "authenticated": false,
                        "note": "no per-model credential: pass actor and role on each call"
                    }));
                };
                let who = b.roles().actor(&i.actor, &i.role)?;
                to_json(json!({
                    "authenticated": true,
                    "actor": who.name,
                    "role": i.role,
                    "capabilities": who.caps
                }))
            },
        ),
        op(
            "card_take",
            "Pull your next piece of work: the card nearest Done that your role can act on. A card in Ready is moved to Building, and the card is claimed for you. Returns {card: null} when nothing is waiting. Then do the work, comment with what you did, and card_move it on.",
            |b, i, a: TakeArgs| {
                let who = acting(b, i, &a.who)?;
                let id = a.id.as_deref().map(id).transpose()?;
                to_json(json!({ "card": b.take_in(&who, id, a.project.as_deref())? }))
            },
        ),
        op(
            "card_get",
            "Fetch one card with its comments.",
            |b, i, a: GetArgs| {
                let c = b.get(id(&a.id)?)?;
                if !in_scope(scope(i), &c.project) {
                    return Err(BoardError::OutOfScope(c.project));
                }
                card(c)
            },
        ),
        op(
            "card_add",
            "Create a card in Backlog.",
            |b, i, a: AddArgs| {
                let who = acting(b, i, &a.who)?;
                card(b.add_in(
                    &who,
                    a.project.as_deref().unwrap_or(DEFAULT_PROJECT),
                    &a.title,
                    a.body.as_deref().unwrap_or(""),
                    a.tags.unwrap_or_default(),
                )?)
            },
        ),
        op(
            "card_assign",
            "Move a card to another project (needs Triage). Drops any claim.",
            |b, i, a: AssignArgs| {
                let who = acting(b, i, &a.who)?;
                card(b.assign(id(&a.id)?, &a.project, &who)?)
            },
        ),
        op(
            "card_move",
            "Move a card. Needs the capability for that transition; leaving building/qa/review needs your claim.",
            |b, i, a: MoveArgs| {
                let who = acting(b, i, &a.who)?;
                card(b.move_card(id(&a.id)?, a.to, &who)?)
            },
        ),
        op(
            "card_claim",
            "Claim a card so only you move it. Fails if someone else holds it.",
            |b, i, a: IdArgs| {
                let who = acting(b, i, &a.who)?;
                card(b.claim(id(&a.id)?, &who)?)
            },
        ),
        op(
            "card_release",
            "Release your claim on a card.",
            |b, i, a: IdArgs| {
                let who = acting(b, i, &a.who)?;
                card(b.release(id(&a.id)?, &who)?)
            },
        ),
        op(
            "card_update",
            "Edit a card's title, body or tags. Pass `rev` (the revision you last read) to refuse the edit if the card has changed since.",
            |b, i, a: UpdateArgs| {
                let who = acting(b, i, &a.who)?;
                card(b.edit_at(
                    id(&a.id)?,
                    &who,
                    a.rev,
                    a.title.as_deref(),
                    a.body.as_deref(),
                    a.tags,
                )?)
            },
        ),
        op(
            "card_comment",
            "Add a comment to a card.",
            |b, i, a: CommentArgs| {
                let who = acting(b, i, &a.who)?;
                card(b.comment(id(&a.id)?, &who, &a.text)?)
            },
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::Roles;
    use crate::store::MemStore;

    fn call(name: &str, args: Value) -> Result<Value, CallError> {
        let board: Shared = Board::new(Box::new(MemStore::default()), Roles::default());
        call_as(&board, None, name, args)
    }

    fn call_as(
        board: &Shared,
        who: Option<&Identity>,
        name: &str,
        args: Value,
    ) -> Result<Value, CallError> {
        ops()
            .iter()
            .find(|o| o.name == name)
            .expect("op")
            .call(board, who, args)
    }

    #[test]
    fn schemas_are_objects_with_properties() {
        for o in ops() {
            assert_eq!(o.schema["type"], "object", "{}", o.name);
        }
        let add = ops()
            .into_iter()
            .find(|o| o.name == "card_add")
            .expect("add");
        assert!(add.schema["properties"]["actor"].is_object());
        assert!(add.schema["properties"]["title"].is_object());
    }

    #[test]
    fn bad_args_and_board_errors_are_distinguished() {
        assert!(matches!(
            call("card_add", json!({"title": 3})),
            Err(CallError::BadArgs(_))
        ));
        assert!(matches!(
            call(
                "card_add",
                json!({"actor": "a", "role": "nope", "title": "t"})
            ),
            Err(CallError::Board(BoardError::Invalid("unknown role")))
        ));
        assert!(matches!(
            call("card_get", json!({"id": "x"})),
            Err(CallError::Board(BoardError::Invalid(_)))
        ));
    }

    #[test]
    fn add_returns_card() {
        let out = call(
            "card_add",
            json!({"actor": "a", "role": "human", "title": "t"}),
        )
        .expect("add");
        assert_eq!(out["card"]["column"], "backlog");
    }

    fn ident(actor: &str, role: &str) -> Identity {
        Identity {
            projects: None,
            actor: actor.into(),
            role: role.into(),
        }
    }

    #[test]
    fn a_credential_fixes_actor_and_role() {
        let board: Shared = Board::new(Box::new(MemStore::default()), Roles::default());
        let dev = ident("claude", "builder");
        // No actor/role needed; the card records the credential's actor.
        let added = call_as(&board, Some(&dev), "card_add", json!({"title": "t"})).expect("add");
        assert_eq!(added["card"]["column"], "backlog");
        // Repeating them exactly is fine; claiming to be someone else is refused.
        assert!(
            call_as(
                &board,
                Some(&dev),
                "card_add",
                json!({"title": "t", "actor": "claude", "role": "builder"})
            )
            .is_ok()
        );
        for lie in [
            json!({"title": "t", "role": "human"}),
            json!({"title": "t", "actor": "nano"}),
        ] {
            assert!(
                matches!(call_as(&board, Some(&dev), "card_add", lie), Err(CallError::Board(BoardError::Invalid(m))) if m.contains("fixed by your credential"))
            );
        }
        // Without a credential, they are required.
        assert!(matches!(
            call_as(&board, None, "card_add", json!({"title": "t"})),
            Err(CallError::Board(BoardError::Invalid("actor is required")))
        ));
    }

    #[test]
    fn whoami_reports_identity_and_capabilities() {
        let board: Shared = Board::new(Box::new(MemStore::default()), Roles::default());
        let me = call_as(
            &board,
            Some(&ident("claude", "builder")),
            "board_whoami",
            json!({}),
        )
        .expect("whoami");
        assert_eq!(
            (
                me["authenticated"].clone(),
                me["actor"].clone(),
                me["capabilities"].clone()
            ),
            (json!(true), json!("claude"), json!(["build"]))
        );
        let anon = call_as(&board, None, "board_whoami", json!({})).expect("whoami");
        assert_eq!(anon["authenticated"], json!(false));
    }

    #[test]
    fn take_pulls_a_ready_card_for_the_credentialed_model() {
        let board: Shared = Board::new(Box::new(MemStore::default()), Roles::default());
        let (human, dev) = (ident("nano", "human"), ident("claude", "builder"));
        let id = call_as(&board, Some(&human), "card_add", json!({"title": "work"})).expect("add")
            ["card"]["id"]
            .as_str()
            .expect("id")
            .to_owned();
        call_as(
            &board,
            Some(&human),
            "card_move",
            json!({"id": id, "to": "ready"}),
        )
        .expect("ready");

        let taken = call_as(&board, Some(&dev), "card_take", json!({})).expect("take");
        assert_eq!(
            (
                taken["card"]["column"].clone(),
                taken["card"]["claim"].clone()
            ),
            (json!("building"), json!("claude"))
        );
        // Nothing else waiting for a builder.
        let again = call_as(
            &board,
            Some(&ident("codex", "builder")),
            "card_take",
            json!({}),
        )
        .expect("take");
        assert_eq!(again["card"], Value::Null);
    }

    #[test]
    fn events_carry_a_cursor_and_tail_starts_a_watcher_at_the_end() {
        let board: Shared = Board::new(Box::new(MemStore::default()), Roles::default());
        let human = ident("nano", "human");
        for t in ["a", "b", "c"] {
            call_as(&board, Some(&human), "card_add", json!({"title": t})).expect("add");
        }
        let page =
            call_as(&board, Some(&human), "board_events", json!({"limit": 2})).expect("events");
        let first = page["events"].as_array().expect("events");
        assert_eq!(first.len(), 2);
        assert_eq!(
            page["cursor"], first[1]["id"],
            "the cursor is the last event returned"
        );

        let rest = call_as(
            &board,
            Some(&human),
            "board_events",
            json!({"after": page["cursor"]}),
        )
        .expect("events");
        assert_eq!(rest["events"].as_array().expect("events").len(), 1);

        // Nothing newer: no events, and the cursor stays where the caller was.
        let none = call_as(
            &board,
            Some(&human),
            "board_events",
            json!({"after": rest["cursor"]}),
        )
        .expect("events");
        assert_eq!(
            (
                none["events"].as_array().map(Vec::len),
                none["cursor"].clone()
            ),
            (Some(0), rest["cursor"].clone())
        );

        // A watcher starts at the very end, however long the log is.
        let tail =
            call_as(&board, Some(&human), "board_events", json!({"tail": true})).expect("tail");
        assert_eq!(
            (
                tail["events"].as_array().map(Vec::len),
                tail["cursor"].clone()
            ),
            (Some(0), rest["cursor"].clone())
        );
        let empty: Shared = Board::new(Box::new(MemStore::default()), Roles::default());
        assert_eq!(
            call_as(&empty, Some(&human), "board_events", json!({"tail": true})).expect("tail")["cursor"],
            Value::Null
        );
    }

    #[test]
    fn an_update_with_a_stale_revision_is_refused() {
        let board: Shared = Board::new(Box::new(MemStore::default()), Roles::default());
        let human = ident("nano", "human");
        let card =
            call_as(&board, Some(&human), "card_add", json!({"title": "t"})).expect("add")["card"]
                .clone();
        let id = card["id"].clone();
        let rev = card["rev"].clone();

        let ok = call_as(
            &board,
            Some(&human),
            "card_update",
            json!({"id": id, "rev": rev, "title": "one"}),
        )
        .expect("current rev");
        assert_eq!(ok["card"]["title"], "one");
        // The same (now stale) revision again: refused, and nothing changed.
        let stale = call_as(
            &board,
            Some(&human),
            "card_update",
            json!({"id": id, "rev": rev, "title": "two"}),
        );
        assert!(
            matches!(stale, Err(CallError::Board(BoardError::Conflict))),
            "{stale:?}"
        );
        assert_eq!(
            call_as(&board, Some(&human), "card_get", json!({"id": id})).expect("get")["card"]["title"],
            "one"
        );
        // Without a revision the edit applies, as before.
        assert!(
            call_as(
                &board,
                Some(&human),
                "card_update",
                json!({"id": id, "title": "three"})
            )
            .is_ok()
        );
    }

    #[test]
    fn a_credential_limited_to_projects_sees_and_takes_only_those() {
        use crate::projects::Projects;
        let projects = Projects::parse(
            r#"{"rl": ["crates/apps/rocket_league"], "orch": ["crates/apps/rusty_orch"]}"#,
        )
        .expect("projects");
        let board: Shared = Board::new(
            Box::new(MemStore::default()) as Box<dyn Store>,
            Roles::default(),
        )
        .with_projects(projects);
        let human = ident("nano", "human");
        let mut dev = ident("claude", "builder");
        dev.projects = Some(vec!["rl".into()]);
        let mut ids = Vec::new();
        for (project, title) in [("rl", "rl card"), ("orch", "orch card")] {
            let out = call_as(
                &board,
                Some(&human),
                "card_add",
                json!({"title": title, "project": project}),
            )
            .expect("add");
            let id = out["card"]["id"].as_str().expect("id").to_owned();
            call_as(
                &board,
                Some(&human),
                "card_move",
                json!({"id": id, "to": "ready"}),
            )
            .expect("ready");
            ids.push(id);
        }

        let titles = |who: &Identity, args: Value| -> Vec<String> {
            let out = call_as(&board, Some(who), "board_list", args).expect("list");
            out["cards"]
                .as_array()
                .expect("cards")
                .iter()
                .map(|c| c["title"].as_str().expect("t").to_owned())
                .collect()
        };
        assert_eq!(
            titles(&dev, json!({})),
            ["rl card"],
            "the other project is invisible"
        );
        assert_eq!(titles(&human, json!({})).len(), 2);
        assert_eq!(titles(&human, json!({"project": "orch"})), ["orch card"]);
        assert!(matches!(
            call_as(&board, Some(&dev), "board_list", json!({"project": "orch"})),
            Err(CallError::Board(BoardError::OutOfScope(_)))
        ));
        assert!(matches!(
            call_as(&board, Some(&dev), "card_get", json!({"id": ids[1]})),
            Err(CallError::Board(BoardError::OutOfScope(_)))
        ));
        assert!(matches!(
            call_as(&board, Some(&dev), "card_take", json!({"id": ids[1]})),
            Err(CallError::Board(BoardError::OutOfScope(_)))
        ));
        let taken = call_as(&board, Some(&dev), "card_take", json!({})).expect("take");
        assert_eq!(taken["card"]["title"], "rl card");
        let listed = call_as(&board, Some(&dev), "board_projects", json!({})).expect("projects");
        assert_eq!(listed["default"], "default");
        let repos = listed["repos"].as_object().expect("repos");
        assert_eq!(
            repos.len(),
            1,
            "a limited credential hears of no other project: {repos:?}"
        );
        assert_eq!(repos["default"].as_object().expect("projects").len(), 1);
        let everything =
            call_as(&board, Some(&human), "board_projects", json!({})).expect("projects");
        assert_eq!(
            everything["repos"]["default"]
                .as_object()
                .expect("projects")
                .len(),
            3
        );
        assert_eq!(
            listed["repos"]["default"]["rl"],
            json!(["crates/apps/rocket_league"])
        );
    }
}
