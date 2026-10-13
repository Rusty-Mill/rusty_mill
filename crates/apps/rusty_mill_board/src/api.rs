//! The operation table. MCP tools and the web API both dispatch through it,
//! so the two surfaces cannot drift: one name, one argument type, one schema.

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::board::Board;
use crate::domain::{BoardError, Column};
use crate::store::Store;

/// The board every surface shares.
pub type Shared = Board<Box<dyn Store>>;

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

/// One named operation.
pub struct Op {
    /// Tool name and web path segment.
    pub name: &'static str,
    /// Shown to agents.
    pub description: &'static str,
    /// JSON Schema of the arguments.
    pub schema: Value,
    run: Box<dyn Fn(&Shared, Value) -> Result<Value, CallError> + Send + Sync>,
}

impl Op {
    /// Run with raw JSON arguments.
    ///
    /// # Errors
    /// [`CallError::BadArgs`] if they don't parse, else whatever the board says.
    pub fn call(&self, board: &Shared, args: Value) -> Result<Value, CallError> {
        (self.run)(board, args)
    }
}

fn op<A>(
    name: &'static str,
    description: &'static str,
    f: fn(&Shared, A) -> Result<Value, BoardError>,
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
        run: Box::new(move |board, args| {
            let args =
                serde_json::from_value(args).map_err(|e| CallError::BadArgs(e.to_string()))?;
            f(board, args).map_err(CallError::Board)
        }),
    }
}

/// Who is acting. The board trusts the caller's claim of role: it is a
/// workflow guard between cooperating agents, not authentication (the
/// transport's bearer token is that).
#[derive(Deserialize, JsonSchema)]
struct Who {
    /// Stable name of the actor, recorded in claims and history.
    actor: String,
    /// Role name; see `board_roles`.
    role: String,
}

#[derive(Deserialize, JsonSchema)]
struct Nothing {}

#[derive(Deserialize, JsonSchema)]
struct ListArgs {
    /// Only this column.
    column: Option<Column>,
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
}

fn id(raw: &str) -> Result<Uuid, BoardError> {
    Uuid::parse_str(raw).map_err(|_| BoardError::Invalid("id is not a UUID"))
}

fn card(c: crate::domain::Card) -> Result<Value, BoardError> {
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
            "List cards, optionally one column. Columns: backlog, ready, building, qa, review, done, blocked.",
            |b, a: ListArgs| to_json(json!({ "cards": b.list(a.column)? })),
        ),
        op(
            "board_roles",
            "List roles and the capabilities each holds.",
            |b, _: Nothing| to_json(json!({ "roles": b.roles().0 })),
        ),
        op(
            "board_events",
            "Poll the change log. Pass the last event id seen as `after` to get only newer events.",
            |b, a: EventsArgs| {
                let after = a.after.as_deref().map(id).transpose()?;
                to_json(json!({ "events": b.events(after, a.limit.unwrap_or(100))? }))
            },
        ),
        op(
            "card_get",
            "Fetch one card with its comments.",
            |b, a: GetArgs| card(b.get(id(&a.id)?)?),
        ),
        op("card_add", "Create a card in Backlog.", |b, a: AddArgs| {
            let who = b.roles().actor(&a.who.actor, &a.who.role)?;
            card(b.add(
                &who,
                &a.title,
                a.body.as_deref().unwrap_or(""),
                a.tags.unwrap_or_default(),
            )?)
        }),
        op(
            "card_move",
            "Move a card. Needs the capability for that transition; leaving building/qa/review needs your claim.",
            |b, a: MoveArgs| {
                let who = b.roles().actor(&a.who.actor, &a.who.role)?;
                card(b.move_card(id(&a.id)?, a.to, &who)?)
            },
        ),
        op(
            "card_claim",
            "Claim a card so only you move it. Fails if someone else holds it.",
            |b, a: IdArgs| {
                let who = b.roles().actor(&a.who.actor, &a.who.role)?;
                card(b.claim(id(&a.id)?, &who)?)
            },
        ),
        op(
            "card_release",
            "Release your claim on a card.",
            |b, a: IdArgs| {
                let who = b.roles().actor(&a.who.actor, &a.who.role)?;
                card(b.release(id(&a.id)?, &who)?)
            },
        ),
        op(
            "card_update",
            "Edit a card's title, body or tags.",
            |b, a: UpdateArgs| {
                let who = b.roles().actor(&a.who.actor, &a.who.role)?;
                card(b.edit(
                    id(&a.id)?,
                    &who,
                    a.title.as_deref(),
                    a.body.as_deref(),
                    a.tags,
                )?)
            },
        ),
        op(
            "card_comment",
            "Add a comment to a card.",
            |b, a: CommentArgs| {
                let who = b.roles().actor(&a.who.actor, &a.who.role)?;
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
        ops()
            .iter()
            .find(|o| o.name == name)
            .expect("op")
            .call(&board, args)
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
}
