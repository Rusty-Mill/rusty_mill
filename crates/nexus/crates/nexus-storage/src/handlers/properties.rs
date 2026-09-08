//! Typed frontmatter property handlers (RFC 0009): `properties_schema`,
//! `properties_set_override`, `properties_get`, `properties_set`,
//! `properties_list`.

use std::collections::BTreeMap;

use nexus_plugins::PluginError;
use serde_json::Value;

use crate::ipc::{
    StorageOk, StoragePathArgs, StoragePropertiesGetResult, StoragePropertiesListArgs,
    StoragePropertiesListResult, StoragePropertiesListRow, StoragePropertiesSchemaResult,
    StoragePropertiesSetArgs, StoragePropertiesSetOverrideArgs, StoragePropertyRow,
};
use crate::properties::{PropertyFilter, PropertyType};
use crate::StorageEngine;

use super::shared::{exec_err, parse_args, to_value};

fn type_map(map: &BTreeMap<String, PropertyType>) -> BTreeMap<String, String> {
    map.iter()
        .map(|(k, v)| (k.clone(), v.as_str().to_string()))
        .collect()
}

fn parse_type(raw: &str, handler: &str) -> Result<PropertyType, PluginError> {
    PropertyType::parse(raw).ok_or_else(|| {
        exec_err(format!(
            "{handler}: unknown property type '{raw}' (expected one of {})",
            PropertyType::ALL
                .iter()
                .map(|t| t.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ))
    })
}

pub(crate) fn schema(engine: &StorageEngine, args: &Value) -> Result<Value, PluginError> {
    let _: serde_json::Map<String, Value> = parse_args(args, "properties_schema")?;
    let schema = engine
        .property_schema()
        .map_err(|e| exec_err(format!("properties_schema: {e}")))?;
    to_value(
        &StoragePropertiesSchemaResult {
            inferred: type_map(&schema.inferred),
            overrides: type_map(&schema.overrides),
        },
        "properties_schema",
    )
}

pub(crate) fn set_override(engine: &StorageEngine, args: &Value) -> Result<Value, PluginError> {
    let StoragePropertiesSetOverrideArgs { key, property_type } =
        parse_args(args, "properties_set_override")?;
    let ty = property_type
        .as_deref()
        .map(|raw| parse_type(raw, "properties_set_override"))
        .transpose()?;
    engine
        .set_property_type_override(&key, ty)
        .map_err(|e| exec_err(format!("properties_set_override '{key}': {e}")))?;
    to_value(&StorageOk { ok: true }, "properties_set_override")
}

pub(crate) fn get(engine: &StorageEngine, args: &Value) -> Result<Value, PluginError> {
    let StoragePathArgs { path } = parse_args(args, "properties_get")?;
    let rows = engine
        .note_properties(&path)
        .map_err(|e| exec_err(format!("properties_get '{path}': {e}")))?
        .into_iter()
        .map(|r| StoragePropertyRow {
            key: r.key,
            property_type: r.property_type.as_str().to_string(),
            value: r.value,
        })
        .collect();
    to_value(&StoragePropertiesGetResult { rows }, "properties_get")
}

pub(crate) fn set(engine: &StorageEngine, args: &Value) -> Result<Value, PluginError> {
    let StoragePropertiesSetArgs {
        path,
        key,
        value,
        property_type,
    } = parse_args(args, "properties_set")?;
    let ty = property_type
        .as_deref()
        .map(|raw| parse_type(raw, "properties_set"))
        .transpose()?;
    engine
        .set_note_property(&path, &key, value.as_ref(), ty)
        .map_err(|e| exec_err(format!("properties_set '{path}' key='{key}': {e}")))?;
    to_value(&StorageOk { ok: true }, "properties_set")
}

pub(crate) fn list(engine: &StorageEngine, args: &Value) -> Result<Value, PluginError> {
    let StoragePropertiesListArgs {
        key,
        value,
        limit,
        offset,
    } = parse_args(args, "properties_list")?;
    let page = engine
        .list_note_properties(&PropertyFilter {
            key,
            value,
            limit,
            offset,
        })
        .map_err(|e| exec_err(format!("properties_list: {e}")))?;
    to_value(
        &StoragePropertiesListResult {
            columns: page.columns,
            rows: page
                .rows
                .into_iter()
                .map(|r| StoragePropertiesListRow {
                    path: r.path,
                    title: r.title,
                    properties: r.properties,
                })
                .collect(),
            total: page.total,
        },
        "properties_list",
    )
}
