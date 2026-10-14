//! The board over its HTTP API (`POST /api/<op>`).

use std::sync::Mutex;
use std::time::Duration;

use rusty_mill_board::domain::{Card, Column};
use rusty_mill_board::http::Http;
use rusty_mill_board::projects::Projects;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::ports::{ApiError, BoardApi, Roles};

/// A board client acting as `actor`.
pub struct HttpBoard {
    base: String,
    token: Option<String>,
    actor: String,
    // One request at a time: calls are short next to the commands they surround.
    http: Mutex<Http>,
}

fn transport(message: impl ToString) -> ApiError {
    ApiError {
        status: 0,
        message: message.to_string(),
    }
}

impl HttpBoard {
    /// `base` is e.g. `http://127.0.0.1:8787`.
    ///
    /// # Errors
    /// If the HTTP client cannot start.
    pub fn new(base: &str, token: Option<String>, actor: &str) -> Result<Self, ApiError> {
        let http = Http::new(Duration::from_secs(30)).map_err(transport)?;
        Ok(Self {
            base: base.trim_end_matches('/').to_owned(),
            token,
            actor: actor.to_owned(),
            http: Mutex::new(http),
        })
    }

    fn call(&self, op: &str, args: Value) -> Result<Value, ApiError> {
        let auth = self.token.as_ref().map(|t| format!("Bearer {t}"));
        let mut headers = Vec::new();
        if let Some(a) = &auth {
            headers.push(("Authorization", a.as_str()));
        }
        let url = format!("{}/api/{op}", self.base);
        let (status, body) = self
            .http
            .lock()
            .map_err(|_| transport("http lock poisoned"))?
            .send("POST", &url, &headers, Some(args.to_string()))
            .map_err(transport)?;
        let json: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
        if (200..300).contains(&status) {
            return Ok(json);
        }
        let message = json["error"]
            .as_str()
            .map_or_else(|| body.chars().take(200).collect(), str::to_owned);
        Err(ApiError { status, message })
    }

    fn act(&self, op: &str, role: &str, mut args: Value) -> Result<Card, ApiError> {
        args["actor"] = json!(self.actor);
        args["role"] = json!(role);
        let out = self.call(op, args)?;
        serde_json::from_value(out["card"].clone()).map_err(transport)
    }
}

impl BoardApi for HttpBoard {
    fn list(&self, column: Column) -> Result<Vec<Card>, ApiError> {
        let out = self.call("board_list", json!({ "column": column }))?;
        serde_json::from_value(out["cards"].clone()).map_err(transport)
    }

    fn get(&self, id: Uuid) -> Result<Card, ApiError> {
        let out = self.call("card_get", json!({ "id": id }))?;
        serde_json::from_value(out["card"].clone()).map_err(transport)
    }

    fn roles(&self) -> Result<Roles, ApiError> {
        let out = self.call("board_roles", json!({}))?;
        serde_json::from_value(out["roles"].clone()).map_err(transport)
    }

    fn projects(&self) -> Result<Projects, ApiError> {
        let out = self.call("board_projects", json!({}))?;
        let listed = serde_json::from_value(out["projects"].clone()).map_err(transport)?;
        Projects::from_listing(listed).map_err(transport)
    }

    fn move_card(&self, role: &str, id: Uuid, to: Column) -> Result<Card, ApiError> {
        self.act("card_move", role, json!({ "id": id, "to": to }))
    }

    fn claim(&self, role: &str, id: Uuid) -> Result<Card, ApiError> {
        self.act("card_claim", role, json!({ "id": id }))
    }

    fn release(&self, role: &str, id: Uuid) -> Result<Card, ApiError> {
        self.act("card_release", role, json!({ "id": id }))
    }

    fn comment(&self, role: &str, id: Uuid, text: &str) -> Result<Card, ApiError> {
        self.act("card_comment", role, json!({ "id": id, "text": text }))
    }
}
