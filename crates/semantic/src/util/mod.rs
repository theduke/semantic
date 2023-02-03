use factdb::{AttributeMeta, DataMap, DbSchema};
use semantic_core::base::AttrChildren;

#[cfg(feature = "archive")]
pub mod archive;

pub mod media;

pub mod api_client;

pub fn json_from_slice<T: serde::de::DeserializeOwned>(
    slice: &[u8],
) -> Result<T, serde_path_to_error::Error<serde_json::Error>> {
    let jd = &mut serde_json::Deserializer::from_slice(slice);
    serde_path_to_error::deserialize(jd)
}

pub fn generate_db_schema_typescript_definitions(
    schema: &DbSchema,
) -> Result<String, anyhow::Error> {
    let ts = factor_tools::typescript::schema_to_typescript(&schema, None)?;
    Ok(ts)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Compression {
    Gzip,
}

pub fn db_items_from_json(value: serde_json::Value) -> Result<Vec<DataMap>, anyhow::Error> {
    match value {
        serde_json::Value::Array(values) => {
            let mut items = Vec::new();
            for value in values {
                let new = db_items_from_json(value)?;
                items.extend(new);
            }
            Ok(items)
        }
        serde_json::Value::Object(map) => db_item_from_json_map(map),
        _ => Err(anyhow::anyhow!("Expected array")),
    }
}

fn db_item_from_json_map(
    mut map: serde_json::Map<String, serde_json::Value>,
) -> Result<Vec<DataMap>, anyhow::Error> {
    let children = map.remove(AttrChildren::QUALIFIED_NAME);
    let map: DataMap = serde_json::from_value(serde_json::Value::Object(map))?;

    let mut items = vec![map];
    if let Some(children) = children {
        let nested = db_items_from_json(children)?;
        items.extend(nested);
    }

    Ok(items)
}
