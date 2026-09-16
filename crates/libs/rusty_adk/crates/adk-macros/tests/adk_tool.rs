//! Integration tests for `#[adk_tool]`.
//!
//! These live outside the proc-macro crate because a proc-macro crate cannot
//! use its own macros — the generated code must be compiled by a consumer.

use adk_core::{Args, InvocationContext, RunConfig, SchemaType, Services, Session};
use adk_macros::adk_tool;
use adk_sessions::InMemorySessionService;
use adk_tools::{invoke_tool, Tool, ToolContext};
use serde_json::json;
use std::sync::Arc;

/// Retrieves the current weather for a city.
#[adk_tool]
async fn get_weather(city: String, unit: Option<String>) -> adk_core::Result<serde_json::Value> {
    Ok(json!({
        "status": "success",
        "report": format!("Sunny in {city}"),
        "unit": unit.unwrap_or_else(|| "Celsius".to_string()),
    }))
}

/// Adds two integers together.
#[adk_tool]
async fn add(a: i64, b: i64) -> adk_core::Result<i64> {
    Ok(a + b)
}

/// Records a value into session state.
#[adk_tool]
async fn remember(key: String, ctx: &ToolContext) -> adk_core::Result<serde_json::Value> {
    ctx.set_state("remembered", key.clone());
    Ok(json!({"status": "success", "stored": key}))
}

fn tool_context() -> ToolContext {
    let services = Services::new(Arc::new(InMemorySessionService::new()));
    ToolContext::new(InvocationContext::new(
        Session::new("s", "app", "u"),
        services,
        RunConfig::default(),
    ))
}

fn args(value: serde_json::Value) -> Args {
    match value {
        serde_json::Value::Object(map) => map,
        _ => panic!("test arguments must be a JSON object"),
    }
}

#[test]
fn the_tool_name_is_the_function_name() {
    assert_eq!(GetWeatherTool.name(), "get_weather");
    assert_eq!(AddTool.name(), "add");
}

#[test]
fn the_doc_comment_becomes_the_description() {
    assert_eq!(
        GetWeatherTool.description(),
        "Retrieves the current weather for a city."
    );
}

#[test]
fn the_schema_is_derived_from_the_signature() {
    let declaration = GetWeatherTool.declaration().unwrap();
    let schema = declaration.parameters.unwrap();

    assert_eq!(schema.schema_type, Some(SchemaType::Object));
    assert_eq!(
        schema.properties["city"].schema_type,
        Some(SchemaType::String)
    );
    // `city` is required; `unit` is Option<..> so it is not.
    assert_eq!(schema.required, vec!["city".to_string()]);
    assert!(schema.properties.contains_key("unit"));
}

#[test]
fn integer_parameters_map_to_integer_schema() {
    let schema = AddTool.declaration().unwrap().parameters.unwrap();
    assert_eq!(
        schema.properties["a"].schema_type,
        Some(SchemaType::Integer)
    );
    let mut required = schema.required.clone();
    required.sort();
    assert_eq!(required, vec!["a".to_string(), "b".to_string()]);
}

#[test]
fn the_injected_tool_context_is_hidden_from_the_model() {
    let schema = RememberTool.declaration().unwrap().parameters.unwrap();
    assert!(schema.properties.contains_key("key"));
    assert!(!schema.properties.contains_key("ctx"));
    assert_eq!(schema.required, vec!["key".to_string()]);
}

#[tokio::test]
async fn the_generated_tool_runs() {
    let ctx = tool_context();
    let result = invoke_tool(&GetWeatherTool, args(json!({"city": "Paris"})), &ctx)
        .await
        .unwrap();
    assert_eq!(result["status"], "success");
    assert_eq!(result["report"], "Sunny in Paris");
    assert_eq!(result["unit"], "Celsius");
}

#[tokio::test]
async fn an_optional_argument_may_be_supplied() {
    let ctx = tool_context();
    let result = invoke_tool(
        &GetWeatherTool,
        args(json!({"city": "Oslo", "unit": "Fahrenheit"})),
        &ctx,
    )
    .await
    .unwrap();
    assert_eq!(result["unit"], "Fahrenheit");
}

#[tokio::test]
async fn a_scalar_return_is_wrapped_under_result() {
    let ctx = tool_context();
    let result = invoke_tool(&AddTool, args(json!({"a": 2, "b": 3})), &ctx)
        .await
        .unwrap();
    assert_eq!(result, json!({"result": 5}));
}

#[tokio::test]
async fn a_missing_required_argument_is_rejected_before_the_body_runs() {
    let ctx = tool_context();
    let err = invoke_tool(&GetWeatherTool, args(json!({})), &ctx)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("city"), "got: {err}");
}

#[tokio::test]
async fn a_wrong_argument_type_is_rejected() {
    let ctx = tool_context();
    let err = invoke_tool(&GetWeatherTool, args(json!({"city": 42})), &ctx)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("expected string"), "got: {err}");
}

#[tokio::test]
async fn the_tool_context_is_injected_and_usable() {
    let ctx = tool_context();
    invoke_tool(&RememberTool, args(json!({"key": "blue"})), &ctx)
        .await
        .unwrap();
    assert_eq!(ctx.state("remembered").unwrap(), json!("blue"));
}

#[test]
fn the_generated_constructor_returns_a_shared_tool() {
    let tool: Arc<dyn Tool> = get_weather_tool();
    assert_eq!(tool.name(), "get_weather");
}

#[tokio::test]
async fn the_original_function_is_still_callable_directly() {
    let value = add(2, 40).await.unwrap();
    assert_eq!(value, 42);
}
