use std::{collections::BTreeMap, fmt, str::FromStr};

use semantic_data::{
    builtin::DEFAULT_COLLECTION,
    bundles::directory::{ATTR_DIRECTORY_NODE_FROM, DIRECTORY_CLASS_ID, DIRECTORY_NODE_CLASS_ID},
    filestore::{
        ATTR_FILE_BYTE_SIZE, ATTR_FILE_CONTENT_HASH_SHA256, ATTR_FILE_FILENAME, FILE_CLASS_ID,
    },
    value::{Object, Value},
};
use semantic_rpc::RpcClient;
use sha2::{Digest as _, Sha256};

const PAGE_SIZE: usize = 1_000;
const ATTR_RELATION_TO: &str = "semantic:relation:to";

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum EntityFormat {
    #[default]
    Json,
    Yaml,
}

impl EntityFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Yaml => "yaml",
        }
    }

    pub(crate) fn serialize(self, object: &Object) -> std::result::Result<Vec<u8>, String> {
        match self {
            Self::Json => serde_json::to_vec_pretty(&Value::Object(object.clone()))
                .map_err(|err| err.to_string()),
            Self::Yaml => serde_yaml::to_string(&Value::Object(object.clone()))
                .map(String::into_bytes)
                .map_err(|err| err.to_string()),
        }
    }

    pub(crate) fn parse(self, bytes: &[u8]) -> std::result::Result<Object, String> {
        let value = match self {
            Self::Json => serde_json::from_slice::<Value>(bytes).map_err(|err| err.to_string())?,
            Self::Yaml => serde_yaml::from_slice::<Value>(bytes).map_err(|err| err.to_string())?,
        };
        match value {
            Value::Object(object) => Ok(object),
            _ => Err("entity document must contain an object".to_string()),
        }
    }
}

impl fmt::Display for EntityFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.extension())
    }
}

impl FromStr for EntityFormat {
    type Err = String;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "json" => Ok(Self::Json),
            "yaml" | "yml" => Ok(Self::Yaml),
            _ => Err(format!(
                "unsupported entity format '{value}' (expected json or yaml)"
            )),
        }
    }
}

#[derive(Clone, Debug)]
pub struct MountConfig {
    /// Scope exposed by this mount. A mount presents one scope; mount another
    /// instance when independent databases need to be browsed simultaneously.
    pub scope_id: Option<String>,
    pub collection: String,
    pub format: EntityFormat,
    pub allow_other: bool,
    pub read_only: bool,
}

impl Default for MountConfig {
    fn default() -> Self {
        Self {
            scope_id: None,
            collection: DEFAULT_COLLECTION.to_string(),
            format: EntityFormat::Json,
            allow_other: false,
            read_only: false,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Entity {
    pub id: String,
    pub object: Object,
}

impl Entity {
    pub fn type_id(&self) -> Option<&str> {
        object_string(&self.object, &["type", "semantic:type"])
    }

    pub fn title(&self) -> &str {
        object_string(&self.object, &["title", "semantic:title"])
            .filter(|value| !value.is_empty())
            .unwrap_or(&self.id)
    }

    pub fn filename(&self) -> Option<&str> {
        object_string(&self.object, &["filename", ATTR_FILE_FILENAME])
    }

    pub fn byte_size(&self) -> u64 {
        object_u64(&self.object, &["byte_size", ATTR_FILE_BYTE_SIZE]).unwrap_or(0)
    }

    pub fn hash(&self) -> Option<&str> {
        object_string(
            &self.object,
            &["content_hash_sha256", ATTR_FILE_CONTENT_HASH_SHA256],
        )
    }

    pub fn is_directory(&self) -> bool {
        self.type_id() == Some(DIRECTORY_CLASS_ID)
    }

    pub fn is_file(&self) -> bool {
        self.type_id() == Some(FILE_CLASS_ID)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct DirectoryLink {
    pub parent: String,
    pub child: String,
}

#[derive(Clone, Debug)]
pub(crate) struct Snapshot {
    pub entities: BTreeMap<String, Entity>,
    pub links: Vec<DirectoryLink>,
}

pub(crate) async fn load_snapshot(
    client: &RpcClient,
    config: &MountConfig,
) -> std::result::Result<Snapshot, String> {
    let mut entities = BTreeMap::new();
    let mut offset = 0;
    loop {
        let query = snapshot_query(&config.collection, offset);
        let rows = run_query(client, config.scope_id.clone(), query).await?;
        let done = rows.len() < PAGE_SIZE;
        for mut object in rows {
            let Some(id) = object_string(&object, &["id", "semantic:id"]).map(str::to_string)
            else {
                continue;
            };
            object
                .entry("id".to_string())
                .or_insert_with(|| Value::String(id.clone()));
            entities.insert(id.clone(), Entity { id, object });
        }
        if done {
            break;
        }
        offset += PAGE_SIZE;
    }

    let links = entities
        .values()
        .filter(|entity| entity.type_id() == Some(DIRECTORY_NODE_CLASS_ID))
        .filter_map(|entity| {
            Some(DirectoryLink {
                parent: object_string(
                    &entity.object,
                    &["directory_from", ATTR_DIRECTORY_NODE_FROM],
                )?
                .to_string(),
                child: object_string(&entity.object, &["directory_to", ATTR_RELATION_TO])?
                    .to_string(),
            })
        })
        .collect();
    Ok(Snapshot { entities, links })
}

fn snapshot_query(collection: &str, offset: usize) -> String {
    format!(
        "SELECT e.* FROM {} AS e ORDER BY e.id ASC LIMIT {PAGE_SIZE} OFFSET {offset} FORMAT qualified",
        sql_ident(collection),
    )
}

async fn run_query(
    client: &RpcClient,
    scope_id: Option<String>,
    query: String,
) -> std::result::Result<Vec<Object>, String> {
    let mut payload = Object::new();
    if let Some(scope_id) = scope_id {
        payload.insert("scope_id", Value::String(scope_id));
    }
    payload.insert("query", Value::String(query));
    payload.insert("format", Value::String("sql".to_string()));
    let response = client
        .invoke_value("semantic.db.query", Value::Object(payload))
        .await
        .map_err(|err| err.to_string())?;
    let Value::Object(response) = response else {
        return Err("query response must be an object".to_string());
    };
    let Some(Value::List(rows)) = response.get("rows") else {
        return Ok(Vec::new());
    };
    Ok(rows
        .iter()
        .filter_map(|row| match row {
            Value::Object(object) => Some(object.clone()),
            _ => None,
        })
        .collect())
}

pub(crate) fn escape_name(value: &str) -> String {
    if value == "." {
        return "%2E".to_string();
    }
    if value == ".." {
        return "%2E%2E".to_string();
    }
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '/' => escaped.push_str("%2F"),
            '%' => escaped.push_str("%25"),
            character if character.is_control() => {
                let mut encoded = [0_u8; 4];
                for byte in character.encode_utf8(&mut encoded).bytes() {
                    escaped.push_str(&format!("%{byte:02X}"));
                }
            }
            _ => escaped.push(character),
        }
    }
    if escaped.is_empty() {
        "%00".to_string()
    } else {
        escaped
    }
}

pub(crate) fn hash_prefix(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    format!("{:02x}", digest[0])
}

pub(crate) fn collision_name(name: &str, id: &str) -> String {
    let digest = Sha256::digest(id.as_bytes());
    format!("{name}~{}", hex::encode(&digest[..3]))
}

pub(crate) fn sql_ident(value: &str) -> String {
    if value
        .chars()
        .all(|character| character == '_' || character.is_ascii_alphanumeric())
    {
        value.to_string()
    } else {
        format!("\"{}\"", value.replace('"', "\"\""))
    }
}

pub(crate) fn object_string<'a>(object: &'a Object, names: &[&str]) -> Option<&'a str> {
    names
        .iter()
        .find_map(|name| object.get(*name).and_then(Value::as_str))
}

fn object_u64(object: &Object, names: &[&str]) -> Option<u64> {
    names.iter().find_map(|name| match object.get(*name) {
        Some(Value::U64(value)) => Some(*value),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_reserved_filesystem_names() {
        assert_eq!(escape_name("entity/a%b"), "entity%2Fa%25b");
        assert_eq!(escape_name("."), "%2E");
        assert_eq!(escape_name(".."), "%2E%2E");
        assert_eq!(escape_name(""), "%00");
        assert_eq!(escape_name("München"), "München");
    }

    #[test]
    fn serializes_entities_in_selected_format() {
        let mut object = Object::new();
        object.insert("id", Value::String("one".to_string()));
        let json = EntityFormat::Json.serialize(&object).unwrap();
        let yaml = EntityFormat::Yaml.serialize(&object).unwrap();
        assert!(String::from_utf8(json).unwrap().contains("\"id\": \"one\""));
        assert!(String::from_utf8(yaml).unwrap().contains("id: one"));
        assert_eq!(EntityFormat::Yaml.parse(b"id: one\n").unwrap(), object);
    }

    #[test]
    fn collision_suffix_is_deterministic() {
        assert_eq!(
            collision_name("Notes", "directory-1"),
            collision_name("Notes", "directory-1")
        );
        assert_ne!(
            collision_name("Notes", "directory-1"),
            collision_name("Notes", "directory-2")
        );
    }

    #[test]
    fn snapshot_query_scans_the_collection_directly() {
        let query = snapshot_query("entities", 2_000);
        assert!(query.contains("FROM entities AS e"));
        assert!(!query.contains("entities._"));
        assert!(query.contains("OFFSET 2000"));
        assert!(query.ends_with("FORMAT qualified"));
    }
}
