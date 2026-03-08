use std::ffi::OsStr;
use std::path::PathBuf;

use clap::{ArgAction, Args};
use semantic_data::value::{Object, Value};
use semantic_db_core::DEFAULT_COLLECTION;
use serde::Deserialize;

use crate::{CliError, CommonArgs, open_db};

#[derive(Debug, Clone, Args)]
pub struct InsertArgs {
    #[clap(flatten)]
    pub common: CommonArgs,
    #[arg(long, default_value = DEFAULT_COLLECTION)]
    pub collection: String,
    #[arg(
        short = 'f',
        long = "file",
        value_name = "FILE",
        action = ArgAction::Append,
        required_unless_present = "json",
        conflicts_with = "json"
    )]
    pub files: Vec<PathBuf>,
    #[arg(
        value_name = "JSON",
        required_unless_present = "files",
        conflicts_with = "files"
    )]
    pub json: Option<String>,
}

pub async fn run(args: InsertArgs) -> std::result::Result<(), CliError> {
    let db = open_db(&args.common.db_uri)?;

    let entities = if let Some(json) = args.json.as_deref() {
        parse_json_entities(json)?
    } else {
        let mut entities = Vec::new();
        for path in &args.files {
            entities.extend(parse_entities_file(path).map_err(|err| {
                CliError::Message(format!("failed to read '{}': {err}", path.display()))
            })?);
        }
        entities
    };

    let mut inserted = 0usize;
    for entity in entities {
        let entity_id = extract_entity_id(&entity)?;
        db.insert(args.collection.as_str(), entity_id, entity)
            .await?;
        inserted = inserted.saturating_add(1);
    }

    println!("inserted {inserted} entities");
    Ok(())
}

fn parse_entities_file(path: &PathBuf) -> std::result::Result<Vec<Object>, CliError> {
    let content = std::fs::read_to_string(path)?;
    let ext = path.extension().and_then(OsStr::to_str);

    match ext {
        Some("yaml") | Some("yml") => parse_yaml_entities(&content),
        Some("json") => parse_json_or_json_lines_entities(&content),
        _ => parse_json_or_json_lines_entities(&content).or_else(|_| parse_yaml_entities(&content)),
    }
}

fn parse_json_entities(input: &str) -> std::result::Result<Vec<Object>, CliError> {
    let value: Value = serde_json::from_str(input)?;
    value_to_entities(value)
}

fn parse_json_or_json_lines_entities(input: &str) -> std::result::Result<Vec<Object>, CliError> {
    match serde_json::from_str::<Value>(input) {
        Ok(value) => value_to_entities(value),
        Err(_) => parse_json_lines_entities(input),
    }
}

fn parse_json_lines_entities(input: &str) -> std::result::Result<Vec<Object>, CliError> {
    let mut entities = Vec::new();
    for (line_no, line) in input.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let value: Value = serde_json::from_str(line).map_err(|err| {
            CliError::Message(format!(
                "invalid jsonlines at line {}: {err}",
                line_no.saturating_add(1)
            ))
        })?;
        entities.extend(value_to_entities(value)?);
    }

    if entities.is_empty() {
        return Err(CliError::Message("input contains no entities".to_string()));
    }
    Ok(entities)
}

fn parse_yaml_entities(input: &str) -> std::result::Result<Vec<Object>, CliError> {
    let mut entities = Vec::new();
    for doc in serde_yaml::Deserializer::from_str(input) {
        let value = Value::deserialize(doc)?;
        entities.extend(value_to_entities(value)?);
    }

    if entities.is_empty() {
        return Err(CliError::Message("input contains no entities".to_string()));
    }
    Ok(entities)
}

fn value_to_entities(value: Value) -> std::result::Result<Vec<Object>, CliError> {
    match value {
        Value::Object(entity) => Ok(vec![entity]),
        Value::List(values) => {
            let mut entities = Vec::new();
            for (index, value) in values.into_iter().enumerate() {
                match value {
                    Value::Object(entity) => entities.push(entity),
                    other => {
                        return Err(CliError::Message(format!(
                            "expected entity object at array index {index}, got {other:?}"
                        )));
                    }
                }
            }
            Ok(entities)
        }
        other => Err(CliError::Message(format!(
            "expected entity object or array of entities, got {other:?}"
        ))),
    }
}

fn extract_entity_id(entity: &Object) -> std::result::Result<String, CliError> {
    match entity.get("id") {
        Some(Value::String(id)) => Ok(id.clone()),
        Some(other) => Err(CliError::Message(format!(
            "entity id must be a string, got {other:?}"
        ))),
        None => Err(CliError::Message(
            "entity is missing required string field 'id'".to_string(),
        )),
    }
}
