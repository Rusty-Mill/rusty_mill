//! Serves Rust ADK tools over the Model Context Protocol.
//!
//! ADK has no language-neutral wire protocol for tools, but every ADK SDK can
//! consume an MCP server — so this is how a Rust tool reaches a Python, Go,
//! TypeScript, Java, or Kotlin agent.
//!
//! # Running
//!
//! ```text
//! cargo run -p mcp-tool-server              # stdio (the default)
//! cargo run -p mcp-tool-server -- --http    # streamable HTTP on 127.0.0.1:8080
//! ```
//!
//! # Consuming it from a Python ADK agent
//!
//! ```python
//! from google.adk.agents import LlmAgent
//! from google.adk.tools.mcp_tool import McpToolset
//! from google.adk.tools.mcp_tool.mcp_session_manager import StdioConnectionParams
//! from mcp import StdioServerParameters
//!
//! root_agent = LlmAgent(
//!     model="gemini-flash-latest",
//!     name="weather_assistant",
//!     instruction="Answer weather questions using the available tools.",
//!     tools=[McpToolset(connection_params=StdioConnectionParams(
//!         server_params=StdioServerParameters(
//!             command="./target/release/mcp-tool-server", args=[],
//!         ),
//!     ))],
//! )
//! ```
//!
//! Over HTTP, point `StreamableHTTPConnectionParams(url="http://127.0.0.1:8080/mcp")`
//! at the same binary started with `--http`.
//!
//! # Trying it by hand
//!
//! ```text
//! printf '%s\n%s\n' \
//!   '{"jsonrpc":"2.0","id":1,"method":"initialize"}' \
//!   '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' \
//!   | cargo run -q -p mcp-tool-server
//! ```

use rusty_adk::core::{Result, Services};
use rusty_adk::mcp::{serve_http, serve_stdio, McpServer};
use rusty_adk::prelude::*;
use serde_json::json;
use std::sync::Arc;

/// Retrieves the current weather for a city.
#[adk_tool(crate = ::rusty_adk::tools)]
async fn get_weather(city: String, unit: Option<String>) -> Result<serde_json::Value> {
    let unit = unit.unwrap_or_else(|| "Celsius".to_string());
    let temperature = match city.to_lowercase().as_str() {
        "paris" => 21,
        "tokyo" => 26,
        "oslo" => 4,
        _ => 18,
    };
    Ok(rusty_adk::tools::success(json!({
        "city": city,
        "temperature": temperature,
        "unit": unit,
        "report": format!("It is {temperature} degrees {unit} in {city}."),
    })))
}

/// Converts a temperature between Celsius and Fahrenheit.
#[adk_tool(crate = ::rusty_adk::tools)]
async fn convert_temperature(value: f64, to: String) -> Result<serde_json::Value> {
    let converted = match to.to_lowercase().as_str() {
        "fahrenheit" | "f" => value * 9.0 / 5.0 + 32.0,
        "celsius" | "c" => (value - 32.0) * 5.0 / 9.0,
        other => {
            // Returning an error result rather than `Err` lets the model read
            // the reason and correct itself.
            return Ok(rusty_adk::tools::error(format!(
                "unknown unit '{other}'; expected Celsius or Fahrenheit"
            )));
        }
    };
    Ok(rusty_adk::tools::success(
        json!({"value": converted, "unit": to}),
    ))
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let use_http = args.iter().any(|a| a == "--http");

    let services = Services::new(Arc::new(InMemorySessionService::new()));
    let server = McpServer::new(
        "rusty-adk-weather",
        vec![get_weather_tool(), convert_temperature_tool()],
        services,
    );

    if use_http {
        let addr = std::env::var("MCP_ADDR").unwrap_or_else(|_| "127.0.0.1:8080".to_string());
        // stderr, not stdout: on the stdio transport stdout carries the
        // protocol, and this binary shares one logging habit across both.
        eprintln!("serving MCP over HTTP at http://{addr}/mcp");
        serve_http(Arc::new(server), &addr, "/mcp").await
    } else {
        eprintln!("serving MCP over stdio ({} tools)", server.tools().len());
        serve_stdio(&server).await
    }
}
