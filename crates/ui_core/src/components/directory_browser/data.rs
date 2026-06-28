use std::collections::BTreeSet;

use semantic_data::bundles::directory::{
    ATTR_CREATED_AT, ATTR_TITLE, ATTR_UPDATED_AT, DIRECTORY_CLASS_ID,
};
use semantic_data::value::{Object, Value};

use super::{
    queries::{
        ENTITIES_COLLECTION, child_directories_query, child_query, parent_query, root_query,
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
    let query = if let Some(root) = root.as_deref() {
        child_query(root, sort, page_size + 1, offset)
    } else {
        root_query(page_size + 1, offset)
    };
    let rows = run_select_query(client, scope_id, query).await?;
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
    let roots = run_select_query(
        client.clone(),
        scope_id.clone(),
        root_query(DEFAULT_TREE_LIMIT, 0),
    )
    .await?;
    let roots = roots.into_iter().map(row_to_item).collect::<Vec<_>>();
    let mut path = BTreeSet::new();
    for item in roots {
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
                .or_else(|| row.get("semantic:base:directory_node:order")),
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
