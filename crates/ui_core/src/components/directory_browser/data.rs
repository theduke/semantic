use std::collections::BTreeSet;

use dioxus::logger::tracing::info;
use semantic_data::bundles::directory::{
    ATTR_CREATED_AT, ATTR_DIRECTORY_NODE_FROM, ATTR_DIRECTORY_NODE_ORDER, ATTR_TITLE,
    ATTR_UPDATED_AT, DIRECTORY_CLASS_ID, DIRECTORY_NODE_CLASS_ID, DIRECTORY_NODE_RELATION_ID,
};
use semantic_data::value::{Object, Value};

use super::{
    queries::{
        ATTR_RELATION_RELATION, ATTR_RELATION_TO, ENTITIES_COLLECTION, child_directories_query,
        child_query, directory_nodes_query, parent_query, root_query,
    },
    types::{
        DirectoryBreadcrumb, DirectoryBrowseItem, DirectoryPage, DirectorySort, DirectoryTreeRow,
    },
};

const DEFAULT_TREE_LIMIT: usize = 200;

pub(super) async fn load_directory_page(
    client: semantic_rpc::RpcClient,
    scope_id: Option<String>,
    root: Option<String>,
    page: usize,
    page_size: usize,
    sort: DirectorySort,
) -> std::result::Result<DirectoryPage, String> {
    let page_size = page_size.max(1);
    let offset = page.saturating_mul(page_size);
    let breadcrumbs = if let Some(root) = root.as_deref() {
        let root_object =
            load_entity(client.clone(), scope_id.clone(), ENTITIES_COLLECTION, root).await?;
        let Some(root_object) = root_object else {
            return Err("Directory not found".to_string());
        };
        if !is_directory_object(&root_object) {
            return Err(format!("{root} is not a directory"));
        }
        load_breadcrumbs(client.clone(), scope_id.clone(), root).await?
    } else {
        BreadcrumbLoad {
            breadcrumbs: Vec::new(),
            cycle: false,
        }
    };
    let rows = if let Some(root) = root.as_deref() {
        run_select_query(
            client,
            scope_id,
            child_query(root, sort, page_size + 1, offset),
        )
        .await?
    } else {
        let roots = load_root_directory_rows(client, scope_id).await?;
        roots
            .into_iter()
            .skip(offset)
            .take(page_size + 1)
            .collect::<Vec<_>>()
    };
    let (items, has_next) = page_items_from_rows(rows, page_size);
    Ok(DirectoryPage {
        items,
        has_next,
        breadcrumbs: breadcrumbs.breadcrumbs,
        breadcrumb_cycle: breadcrumbs.cycle,
    })
}

pub(super) async fn load_tree_rows(
    client: semantic_rpc::RpcClient,
    scope_id: Option<String>,
    expanded: BTreeSet<String>,
) -> std::result::Result<Vec<DirectoryTreeRow>, String> {
    let mut rows = Vec::new();
    let roots = load_root_directory_rows(client.clone(), scope_id.clone()).await?;
    let roots = roots.into_iter().map(row_to_item).collect::<Vec<_>>();
    let mut path = BTreeSet::new();
    for item in roots.into_iter().take(DEFAULT_TREE_LIMIT) {
        push_tree_row(
            &mut rows,
            client.clone(),
            scope_id.clone(),
            item,
            &expanded,
            &mut path,
            0,
        )
        .await?;
    }
    Ok(rows)
}

async fn load_root_directory_rows(
    client: semantic_rpc::RpcClient,
    scope_id: Option<String>,
) -> std::result::Result<Vec<Object>, String> {
    let child_ids = load_directory_child_ids(client.clone(), scope_id.clone()).await?;
    let rows = run_select_query(client, scope_id, root_query()).await?;
    let raw_count = rows.len();
    let roots = rows
        .into_iter()
        .filter(|row| {
            row.get("id")
                .and_then(Value::as_str)
                .is_some_and(|id| !child_ids.contains(id))
        })
        .collect::<Vec<_>>();
    info!(
        raw_directory_count = raw_count,
        child_id_count = child_ids.len(),
        root_directory_count = roots.len(),
        "directory browser root directories filtered"
    );
    Ok(roots)
}

async fn load_directory_child_ids(
    client: semantic_rpc::RpcClient,
    scope_id: Option<String>,
) -> std::result::Result<BTreeSet<String>, String> {
    let rows = run_select_query(client, scope_id, directory_nodes_query()).await?;
    Ok(rows
        .iter()
        .filter_map(|row| {
            row.get("directory_to")
                .or_else(|| row.get(ATTR_RELATION_TO))
                .or_else(|| row.get("to"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .collect())
}

pub(super) async fn move_directory_item(
    client: semantic_rpc::RpcClient,
    scope_id: Option<String>,
    item_id: String,
    target_directory_id: String,
) -> std::result::Result<(), String> {
    if item_id == target_directory_id {
        return Err("Cannot move an item into itself".to_string());
    }

    let Some(item) = load_entity(
        client.clone(),
        scope_id.clone(),
        ENTITIES_COLLECTION,
        &item_id,
    )
    .await?
    else {
        return Err("Item not found".to_string());
    };
    let Some(target) = load_entity(
        client.clone(),
        scope_id.clone(),
        ENTITIES_COLLECTION,
        &target_directory_id,
    )
    .await?
    else {
        return Err("Target directory not found".to_string());
    };
    if !is_directory_object(&target) {
        return Err("Target must be a directory".to_string());
    }
    if is_directory_object(&item)
        && target_has_ancestor(
            client.clone(),
            scope_id.clone(),
            &target_directory_id,
            &item_id,
        )
        .await?
    {
        return Err("Cannot move a directory into one of its descendants".to_string());
    }

    let existing_parent = load_parent_link(client.clone(), scope_id.clone(), &item_id).await?;
    if existing_parent.as_ref().map(|link| link.parent_id.as_str())
        == Some(target_directory_id.as_str())
    {
        return Ok(());
    }

    let mut operations = Vec::new();
    if let Some(link) = existing_parent.as_ref() {
        operations.push(Value::Object(batch_delete_operation(
            ENTITIES_COLLECTION.to_string(),
            link.node_id.clone(),
        )));
    }
    operations.push(Value::Object(batch_upsert_operation(
        ENTITIES_COLLECTION.to_string(),
        directory_node_id(&target_directory_id, &item_id),
        directory_node_object(
            &target_directory_id,
            &item_id,
            existing_parent.and_then(|link| link.order).unwrap_or(0),
        ),
    )));

    let mut payload = Object::new();
    if let Some(scope_id) = scope_id {
        payload.insert("scope_id", Value::String(scope_id));
    }
    payload.insert("operations", Value::List(operations));
    client
        .invoke_value("semantic.db.batch", Value::Object(payload))
        .await
        .map(|_| ())
        .map_err(|err| err.to_string())
}

async fn push_tree_row(
    rows: &mut Vec<DirectoryTreeRow>,
    client: semantic_rpc::RpcClient,
    scope_id: Option<String>,
    item: DirectoryBrowseItem,
    expanded: &BTreeSet<String>,
    path: &mut BTreeSet<String>,
    depth: usize,
) -> std::result::Result<(), String> {
    let cycle = path.contains(&item.id);
    rows.push(DirectoryTreeRow {
        item: item.clone(),
        depth,
        cycle,
    });
    if cycle || !expanded.contains(&item.id) {
        return Ok(());
    }
    path.insert(item.id.clone());
    let children = run_select_query(
        client.clone(),
        scope_id.clone(),
        child_directories_query(&item.id, DEFAULT_TREE_LIMIT, 0),
    )
    .await?;
    for child in children.into_iter().map(row_to_item) {
        Box::pin(push_tree_row(
            rows,
            client.clone(),
            scope_id.clone(),
            child,
            expanded,
            path,
            depth + 1,
        ))
        .await?;
    }
    path.remove(&item.id);
    Ok(())
}

struct ParentLink {
    node_id: String,
    parent_id: String,
    order: Option<u64>,
}

async fn load_parent_link(
    client: semantic_rpc::RpcClient,
    scope_id: Option<String>,
    child_id: &str,
) -> std::result::Result<Option<ParentLink>, String> {
    let rows = run_select_query(client, scope_id, parent_query(child_id)).await?;
    Ok(rows.first().and_then(parent_link_from_row))
}

fn parent_link_from_row(row: &Object) -> Option<ParentLink> {
    Some(ParentLink {
        node_id: row.get("id").and_then(Value::as_str)?.to_string(),
        parent_id: row
            .get("directory_from")
            .and_then(Value::as_str)?
            .to_string(),
        order: value_as_u64(
            row.get("directory_order")
                .or_else(|| row.get("order"))
                .or_else(|| row.get(ATTR_DIRECTORY_NODE_ORDER)),
        ),
    })
}

async fn target_has_ancestor(
    client: semantic_rpc::RpcClient,
    scope_id: Option<String>,
    target_id: &str,
    ancestor_id: &str,
) -> std::result::Result<bool, String> {
    let mut current = target_id.to_string();
    for _ in 0..64 {
        let Some(parent) = load_parent_link(client.clone(), scope_id.clone(), &current).await?
        else {
            return Ok(false);
        };
        if parent.parent_id == ancestor_id {
            return Ok(true);
        }
        current = parent.parent_id;
    }
    Err("Directory ancestry is too deep or cyclic".to_string())
}

fn directory_node_object(parent_id: &str, child_id: &str, order: u64) -> Object {
    let mut object = Object::new();
    object.insert("id", Value::String(directory_node_id(parent_id, child_id)));
    object.insert("type", Value::String(DIRECTORY_NODE_CLASS_ID.to_string()));
    object.insert(
        ATTR_RELATION_RELATION,
        Value::String(DIRECTORY_NODE_RELATION_ID.to_string()),
    );
    object.insert(
        ATTR_DIRECTORY_NODE_FROM,
        Value::String(parent_id.to_string()),
    );
    object.insert(ATTR_RELATION_TO, Value::String(child_id.to_string()));
    object.insert(ATTR_DIRECTORY_NODE_ORDER, Value::U64(order));
    object
}

fn directory_node_id(parent_id: &str, child_id: &str) -> String {
    format!(
        "semantic:directory_node:{}:{}",
        hex_id_part(parent_id),
        hex_id_part(child_id)
    )
}

fn hex_id_part(value: &str) -> String {
    value
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn batch_delete_operation(collection: String, id: String) -> Object {
    let mut operation = Object::new();
    operation.insert("kind", Value::String("delete_by_id".to_string()));
    operation.insert("collection", Value::String(collection));
    operation.insert("id", Value::String(id));
    operation
}

fn batch_upsert_operation(collection: String, id: String, object: Object) -> Object {
    let mut operation = Object::new();
    operation.insert("kind", Value::String("upsert".to_string()));
    operation.insert("collection", Value::String(collection));
    operation.insert("id", Value::String(id));
    operation.insert("object", Value::Object(object));
    operation
}

struct BreadcrumbLoad {
    breadcrumbs: Vec<DirectoryBreadcrumb>,
    cycle: bool,
}

async fn load_breadcrumbs(
    client: semantic_rpc::RpcClient,
    scope_id: Option<String>,
    root: &str,
) -> std::result::Result<BreadcrumbLoad, String> {
    let mut ids = vec![root.to_string()];
    let mut seen = BTreeSet::from([root.to_string()]);
    let mut current = root.to_string();
    let mut cycle = false;
    for _ in 0..64 {
        let rows =
            run_select_query(client.clone(), scope_id.clone(), parent_query(&current)).await?;
        let Some(parent) = rows
            .first()
            .and_then(|row| row.get("directory_from"))
            .and_then(Value::as_str)
            .map(str::to_string)
        else {
            break;
        };
        if !seen.insert(parent.clone()) {
            cycle = true;
            break;
        }
        ids.push(parent.clone());
        current = parent;
    }
    ids.reverse();
    let mut breadcrumbs = Vec::new();
    for id in ids {
        let object =
            load_entity(client.clone(), scope_id.clone(), ENTITIES_COLLECTION, &id).await?;
        let Some(object) = object else {
            continue;
        };
        if !is_directory_object(&object) {
            continue;
        }
        breadcrumbs.push(DirectoryBreadcrumb {
            title: object_title(&object, &id),
            id,
        });
    }
    Ok(BreadcrumbLoad { breadcrumbs, cycle })
}

async fn load_entity(
    client: semantic_rpc::RpcClient,
    scope_id: Option<String>,
    collection: &str,
    id: &str,
) -> std::result::Result<Option<Object>, String> {
    let mut payload = Object::new();
    if let Some(scope_id) = scope_id {
        payload.insert("scope_id", Value::String(scope_id));
    }
    payload.insert("collection", Value::String(collection.to_string()));
    payload.insert("id", Value::String(id.to_string()));
    let response = client
        .invoke_value("semantic.db.get", Value::Object(payload))
        .await
        .map_err(|err| err.to_string())?;
    match response {
        Value::Null | Value::Void => Ok(None),
        Value::Object(mut response) => match response.remove("object") {
            Some(Value::Object(mut object)) => {
                if !object.contains_key("id")
                    && let Some(Value::String(id)) = response.remove("id")
                {
                    object.insert("id", Value::String(id));
                }
                Ok(Some(object))
            }
            _ => Err("get response missing object".to_string()),
        },
        _ => Err("get response must be an object or null".to_string()),
    }
}

async fn run_select_query(
    client: semantic_rpc::RpcClient,
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
    let Value::Object(object) = response else {
        return Err("query response must be an object".to_string());
    };
    let Some(Value::List(rows)) = object.get("rows") else {
        return Ok(Vec::new());
    };
    Ok(rows
        .iter()
        .filter_map(|row| match row {
            Value::Object(row) => Some(row.clone()),
            _ => None,
        })
        .collect())
}

pub(super) fn page_items_from_rows(
    rows: Vec<Object>,
    page_size: usize,
) -> (Vec<DirectoryBrowseItem>, bool) {
    let has_next = rows.len() > page_size;
    let items = rows
        .into_iter()
        .take(page_size)
        .map(row_to_item)
        .collect::<Vec<_>>();
    (items, has_next)
}

fn row_to_item(row: Object) -> DirectoryBrowseItem {
    let id = row
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let type_id = row.get("type").and_then(Value::as_str).map(str::to_string);
    DirectoryBrowseItem {
        title: object_title(&row, &id),
        is_directory: is_directory_object(&row),
        order: value_as_u64(
            row.get("directory_order")
                .or_else(|| row.get("order"))
                .or_else(|| row.get(ATTR_DIRECTORY_NODE_ORDER)),
        ),
        created_at: object_string(&row, &["created_at", ATTR_CREATED_AT]).map(str::to_string),
        updated_at: object_string(&row, &["updated_at", ATTR_UPDATED_AT]).map(str::to_string),
        collection: ENTITIES_COLLECTION.to_string(),
        object: row,
        id,
        type_id,
    }
}

pub(super) fn is_directory_object(object: &Object) -> bool {
    object
        .get("type")
        .and_then(Value::as_str)
        .is_some_and(|type_id| type_id == DIRECTORY_CLASS_ID || type_id == "Directory")
}

fn object_title(object: &Object, fallback: &str) -> String {
    object_string(object, &["title", ATTR_TITLE])
        .unwrap_or(fallback)
        .to_string()
}

fn object_string<'a>(object: &'a Object, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| object.get(*key).and_then(Value::as_str))
}

fn value_as_u64(value: Option<&Value>) -> Option<u64> {
    match value {
        Some(Value::U64(value)) => Some(*value),
        Some(Value::U32(value)) => Some(u64::from(*value)),
        Some(Value::U16(value)) => Some(u64::from(*value)),
        Some(Value::U8(value)) => Some(u64::from(*value)),
        Some(Value::I64(value)) => (*value).try_into().ok(),
        Some(Value::I32(value)) => (*value).try_into().ok(),
        Some(Value::I16(value)) => (*value).try_into().ok(),
        Some(Value::I8(value)) => (*value).try_into().ok(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use semantic_data::bundles::directory::DIRECTORY_CLASS_ID;

    use super::*;

    #[test]
    fn page_items_detects_next_page() {
        let rows = vec![Object::new(), Object::new(), Object::new()];
        let (items, has_next) = page_items_from_rows(rows, 2);
        assert_eq!(items.len(), 2);
        assert!(has_next);
    }

    #[test]
    fn directory_detection_uses_directory_class_id() {
        let mut object = Object::new();
        object.insert("type", Value::String(DIRECTORY_CLASS_ID.to_string()));
        assert!(is_directory_object(&object));
    }
}
