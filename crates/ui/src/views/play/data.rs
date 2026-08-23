use std::collections::{BTreeSet, VecDeque};

use semantic_data::{
    builtin::DEFAULT_COLLECTION,
    bundles::directory::{
        ATTR_DIRECTORY_NODE_FROM, ATTR_DIRECTORY_NODE_ORDER, DIRECTORY_CLASS_ID,
        DIRECTORY_NODE_RELATION_ID,
    },
    filestore::{ATTR_FILE_MEDIA_DURATION, ATTR_FILE_MIME_TYPE, ATTR_TITLE},
    value::{Object, Value},
};
use semantic_rpc::RpcClient;
use semantic_ui_core::{EntityTarget, MediaKind, media_kind_for_object};

use super::{
    query::{PlaylistFilter, page_size, playlist_query, sql_ident, sql_string},
    state::QueueEntry,
};

const MAX_RESULTS: usize = 100_000;
const MAX_EXPANSION_DEPTH: usize = 32;
const ATTR_RELATION_RELATION: &str = "semantic:relation:relation";
const ATTR_RELATION_TO: &str = "semantic:relation:to";

#[derive(Clone, Debug, PartialEq)]
pub struct PlaylistLoad {
    pub entries: Vec<QueueEntry>,
    pub warning: Option<String>,
}

pub async fn load_playlist(
    client: RpcClient,
    scope_id: Option<String>,
    filter: PlaylistFilter,
) -> std::result::Result<PlaylistLoad, String> {
    let mut rows = Vec::new();
    if filter.advanced_sql {
        let query = playlist_query(&filter, 0)?;
        rows = run_query(client.clone(), scope_id.clone(), query).await?;
        rows.truncate(MAX_RESULTS);
    } else {
        let mut offset = 0;
        loop {
            let page = run_query(
                client.clone(),
                scope_id.clone(),
                playlist_query(&filter, offset)?,
            )
            .await?;
            let done = page.len() < page_size();
            rows.extend(page);
            if done || rows.len() >= MAX_RESULTS {
                rows.truncate(MAX_RESULTS);
                break;
            }
            offset = offset.saturating_add(page_size());
        }
    }

    if filter.expand_to_media {
        expand_rows(client, scope_id, filter.collection, rows).await
    } else {
        Ok(PlaylistLoad {
            warning: (rows.len() == MAX_RESULTS)
                .then(|| format!("Playlist was limited to {MAX_RESULTS} items")),
            entries: rows
                .into_iter()
                .filter_map(|object| queue_entry(&object, &filter.collection))
                .collect(),
        })
    }
}

pub async fn get_object(
    client: RpcClient,
    scope_id: Option<String>,
    target: EntityTarget,
) -> std::result::Result<Object, String> {
    let mut payload = Object::new();
    if let Some(scope_id) = scope_id {
        payload.insert("scope_id", Value::String(scope_id));
    }
    payload.insert(
        "collection",
        Value::String(target.collection_or_default().to_string()),
    );
    payload.insert("id", Value::String(target.id.clone()));
    let response = client
        .invoke_value("semantic.db.get", Value::Object(payload))
        .await
        .map_err(|error| error.to_string())?;
    let Value::Object(mut response) = response else {
        return Err(format!("Entity {} no longer exists", target.id));
    };
    let Some(Value::Object(mut object)) = response.remove("object") else {
        return Err("Entity response did not contain an object".to_string());
    };
    if !object.contains_key("id") {
        object.insert("id", Value::String(target.id));
    }
    Ok(object)
}

async fn expand_rows(
    client: RpcClient,
    scope_id: Option<String>,
    collection: String,
    rows: Vec<Object>,
) -> std::result::Result<PlaylistLoad, String> {
    let mut queue = VecDeque::new();
    let mut emitted = BTreeSet::new();
    let mut visited = BTreeSet::new();
    let mut entries = Vec::new();
    for row in rows {
        let kind = media_kind_for_object(&row);
        if matches!(kind, MediaKind::Image | MediaKind::Audio | MediaKind::Video) {
            if let Some(entry) = queue_entry(&row, &collection) {
                if emitted.insert(entry.target.id.clone()) {
                    entries.push(entry);
                }
            }
        } else if object_string(&row, &["type"]) == Some(DIRECTORY_CLASS_ID) {
            if let Some(id) = object_string(&row, &["id"]) {
                queue.push_back((id.to_string(), 0_usize));
            }
        } else if let Some(entry) = queue_entry(&row, &collection) {
            entries.push(entry);
        }
    }

    let mut warning = None;
    while let Some((directory_id, depth)) = queue.pop_front() {
        if !visited.insert(directory_id.clone()) {
            continue;
        }
        if depth >= MAX_EXPANSION_DEPTH {
            warning = Some(format!("Expansion stopped at depth {MAX_EXPANSION_DEPTH}"));
            continue;
        }
        let mut offset = 0;
        loop {
            let children = run_query(
                client.clone(),
                scope_id.clone(),
                directory_children_query(&directory_id, page_size(), offset),
            )
            .await?;
            let done = children.len() < page_size();
            for child in children {
                let kind = media_kind_for_object(&child);
                if matches!(kind, MediaKind::Image | MediaKind::Audio | MediaKind::Video) {
                    if let Some(entry) = queue_entry(&child, &collection) {
                        if emitted.insert(entry.target.id.clone()) {
                            entries.push(entry);
                        }
                    }
                } else if object_string(&child, &["type"]) == Some(DIRECTORY_CLASS_ID) {
                    if let Some(id) = object_string(&child, &["id"]) {
                        queue.push_back((id.to_string(), depth + 1));
                    }
                }
                if entries.len() >= MAX_RESULTS {
                    warning = Some(format!("Expansion was limited to {MAX_RESULTS} items"));
                    return Ok(PlaylistLoad { entries, warning });
                }
            }
            if done {
                break;
            }
            offset = offset.saturating_add(page_size());
        }
    }
    Ok(PlaylistLoad { entries, warning })
}

fn directory_children_query(parent_id: &str, limit: usize, offset: usize) -> String {
    format!(
        "SELECT child.*, n.{node_order} AS directory_order FROM {entities} AS n INNER JOIN {entities}._ AS child ON n.{relation_to} = child.id WHERE n.{relation_relation} = {node_relation} AND n.{node_from} = {parent_id} ORDER BY n.{node_order} ASC, child.title ASC, child.id ASC LIMIT {limit} OFFSET {offset} FORMAT qualified",
        node_order = sql_ident(ATTR_DIRECTORY_NODE_ORDER),
        entities = sql_ident(DEFAULT_COLLECTION),
        relation_to = sql_ident(ATTR_RELATION_TO),
        relation_relation = sql_ident(ATTR_RELATION_RELATION),
        node_relation = sql_string(DIRECTORY_NODE_RELATION_ID),
        node_from = sql_ident(ATTR_DIRECTORY_NODE_FROM),
        parent_id = sql_string(parent_id),
    )
}

async fn run_query(
    client: RpcClient,
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
        .map_err(|error| error.to_string())?;
    let Value::Object(response) = response else {
        return Err("Query response must be an object".to_string());
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

fn queue_entry(object: &Object, collection: &str) -> Option<QueueEntry> {
    let id = object_string(object, &["id"])?.to_string();
    let title = object_string(object, &["title", "semantic_title", ATTR_TITLE])
        .filter(|title| !title.is_empty())
        .unwrap_or(&id)
        .to_string();
    let mime_type = object_string(object, &["mime_type", ATTR_FILE_MIME_TYPE]).map(str::to_string);
    let mut kind_object = object.clone();
    if let Some(mime_type) = &mime_type {
        kind_object.insert("mime_type", Value::String(mime_type.clone()));
    }
    Some(QueueEntry {
        occurrence_id: 0,
        target: EntityTarget::new(Some(collection.to_string()), id),
        title,
        class_id: object_string(object, &["type"]).map(str::to_string),
        media_kind: media_kind_for_object(&kind_object),
        mime_type,
        known_duration_seconds: object
            .get("media_duration")
            .or_else(|| object.get(ATTR_FILE_MEDIA_DURATION))
            .and_then(Value::as_f64),
    })
}

fn object_string<'a>(object: &'a Object, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| object.get(*key).and_then(Value::as_str))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_entry_uses_current_qualified_mime_attribute() {
        let mut object = Object::new();
        object.insert("id", Value::String("file-1".to_string()));
        object.insert(ATTR_TITLE, Value::String("Picture".to_string()));
        object.insert(ATTR_FILE_MIME_TYPE, Value::String("image/png".to_string()));
        let entry = queue_entry(&object, DEFAULT_COLLECTION).unwrap();
        assert_eq!(entry.title, "Picture");
        assert_eq!(entry.media_kind, MediaKind::Image);
    }

    #[test]
    fn directory_expansion_query_is_stable_and_paged() {
        let query = directory_children_query("a'b", 500, 1000);
        assert!(query.contains(
            "ORDER BY n.\"semantic:base:directory_node:order\" ASC, child.title ASC, child.id ASC"
        ));
        assert!(query.contains("'a''b'"));
        assert!(query.contains("LIMIT 500 OFFSET 1000"));
    }
}
