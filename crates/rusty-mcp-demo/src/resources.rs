//! Resources exposed by the demo server.
//!
//! Three shapes worth seeing: fixed text, content generated per read, and a
//! templated family where the URI carries a variable.

use rmcp::model::{ErrorData, Resource, ResourceContents, ResourceTemplate};
use rusty_mcp::resources::{ReadRequest, ResourceRegistry};

/// Tables the `db://tables/{table}` template will serve.
const TABLES: &[(&str, &[&str])] = &[
    ("users", &["id", "email", "created_at"]),
    ("orders", &["id", "user_id", "total_cents"]),
];

/// Build the demo registry.
pub fn registry() -> ResourceRegistry {
    ResourceRegistry::new()
        // Fixed content, known at startup.
        .with_text(
            Resource::new("config://demo", "demo-config")
                .with_title("Demo configuration")
                .with_description("Static configuration for the demo server.")
                .with_mime_type("application/json"),
            r#"{"greeting":"hello","tools":["add","divide","slugify","text_stats","countdown"]}"#,
        )
        // Generated per read, so the value is never stale.
        .with_reader(
            Resource::new("status://uptime", "uptime")
                .with_description("How long this process has been running.")
                .with_mime_type("text/plain"),
            |req: ReadRequest| async move {
                let uptime = crate::server::process_uptime();
                Ok(vec![ResourceContents::text(
                    format!("{} seconds", uptime.as_secs()),
                    req.uri.clone(),
                )])
            },
        )
        // One template standing in for a family of resources.
        .with_template(
            ResourceTemplate::new("db://tables/{table}", "table-schema")
                .with_description("Column names for a demo table.")
                .with_mime_type("application/json"),
            |req: ReadRequest| async move {
                let name = req.param("table").unwrap_or_default();

                let columns = TABLES
                    .iter()
                    .find(|(table, _)| *table == name)
                    .map(|(_, columns)| *columns)
                    // An unknown table is a bad parameter, not a server fault.
                    .ok_or_else(|| {
                        ErrorData::invalid_params(format!("no such table `{name}`"), None)
                    })?;

                let body = serde_json::json!({ "table": name, "columns": columns });
                Ok(vec![ResourceContents::text(
                    serde_json::to_string(&body).unwrap_or_default(),
                    req.uri.clone(),
                )])
            },
        )
}
