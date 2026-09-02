use std::collections::{BTreeMap, BTreeSet};

use dioxus::logger::tracing::info;
use semantic_data::bundles::directory::{
    ATTR_CREATED_AT, ATTR_DIRECTORY_NODE_FROM, ATTR_DIRECTORY_NODE_ORDER, ATTR_TITLE,
    ATTR_UPDATED_AT, DIRECTORY_CLASS_ID, DIRECTORY_NODE_CLASS_ID, DIRECTORY_NODE_RELATION_ID,
};
use semantic_data::value::{DateTime, Object, Value};

use super::{
    queries::{
        ATTR_RELATION_RELATION, ATTR_RELATION_TO, ENTITIES_COLLECTION, addable_entities_query,
        child_directories_query, child_ids_query, child_links_query, child_query,
        directory_nodes_query, directory_outgoing_links_query, file_tree_items_query,
        max_child_order_query, parent_count_query, parent_links_query, parent_query, root_query,
    },
    types::{
        DirectoryBreadcrumb, DirectoryBrowseItem, DirectoryPage, DirectorySort, DirectoryTreeRow,
    },
};

const DEFAULT_TREE_LIMIT: usize = 200;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct UnlinkOutcome {
    pub removed_item_ids: Vec<String>,
    pub orphaned_directory_ids: Vec<String>,
    pub requires_confirmation: bool,
}

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

pub(super) async fn load_file_tree_rows(
    client: semantic_rpc::RpcClient,
    scope_id: Option<String>,
    show_files: bool,
) -> std::result::Result<Vec<DirectoryTreeRow>, String> {
    let directory_rows = run_select_query(client.clone(), scope_id.clone(), root_query()).await?;
    let mut items = directory_rows
        .into_iter()
        .map(row_to_item)
        .map(|item| (item.id.clone(), item))
        .collect::<BTreeMap<_, _>>();
    if show_files {
        let linked_items =
            run_select_query(client.clone(), scope_id.clone(), file_tree_items_query()).await?;
        for item in linked_items.into_iter().map(row_to_item) {
            items.entry(item.id.clone()).or_insert(item);
        }
    }
    let link_rows = run_select_query(client, scope_id, directory_nodes_query()).await?;
    let mut children = BTreeMap::<String, Vec<(u64, String)>>::new();
    let mut child_ids = BTreeSet::new();
    for link in link_rows.iter().filter_map(directory_link_from_row) {
        let (Some(parent_id), Some(child_id)) = (link.parent_id, link.child_id) else {
            continue;
        };
        if items.contains_key(&parent_id) && items.contains_key(&child_id) {
            child_ids.insert(child_id.clone());
            children
                .entry(parent_id)
                .or_default()
                .push((link.order.unwrap_or(u64::MAX), child_id));
        }
    }
    for child_rows in children.values_mut() {
        child_rows.sort_by(|left, right| {
            left.0.cmp(&right.0).then_with(|| {
                let left_item = items.get(&left.1);
                let right_item = items.get(&right.1);
                left_item
                    .map(|item| (&item.title, &item.id))
                    .cmp(&right_item.map(|item| (&item.title, &item.id)))
            })
        });
    }

    let mut root_ids = items
        .keys()
        .filter(|id| {
            !child_ids.contains(*id) && items.get(*id).is_some_and(|item| item.is_directory)
        })
        .cloned()
        .collect::<Vec<_>>();
    root_ids.sort_by_key(|id| {
        items
            .get(id)
            .map(|item| (item.title.clone(), item.id.clone()))
    });

    let mut rows = Vec::new();
    let mut emitted = BTreeSet::new();
    let mut path = BTreeSet::new();
    for root_id in root_ids {
        append_picker_tree_rows(
            &mut rows,
            &items,
            &children,
            &root_id,
            0,
            &mut path,
            &mut emitted,
        );
    }
    for id in items.keys() {
        if !emitted.contains(id) {
            append_picker_tree_rows(&mut rows, &items, &children, id, 0, &mut path, &mut emitted);
        }
    }
    Ok(rows)
}

fn append_picker_tree_rows(
    rows: &mut Vec<DirectoryTreeRow>,
    directories: &BTreeMap<String, DirectoryBrowseItem>,
    children: &BTreeMap<String, Vec<(u64, String)>>,
    id: &str,
    depth: usize,
    path: &mut BTreeSet<String>,
    emitted: &mut BTreeSet<String>,
) {
    let Some(item) = directories.get(id).cloned() else {
        return;
    };
    if path.contains(id) {
        rows.push(DirectoryTreeRow {
            item,
            depth,
            cycle: true,
        });
        return;
    }
    if !emitted.insert(id.to_string()) {
        return;
    }
    rows.push(DirectoryTreeRow {
        item,
        depth,
        cycle: false,
    });
    path.insert(id.to_string());
    if let Some(child_rows) = children.get(id) {
        for (_, child_id) in child_rows {
            append_picker_tree_rows(
                rows,
                directories,
                children,
                child_id,
                depth + 1,
                path,
                emitted,
            );
        }
    }
    path.remove(id);
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

pub(super) async fn create_directory(
    client: semantic_rpc::RpcClient,
    scope_id: Option<String>,
    parent: Option<String>,
    title: String,
) -> std::result::Result<String, String> {
    let title = title.trim();
    if title.is_empty() {
        return Err("Directory title is required".to_string());
    }
    if let Some(parent_id) = parent.as_deref() {
        validate_directory(client.clone(), scope_id.clone(), parent_id).await?;
    }

    let id = format!("directory-{}", unix_time_millis());
    let now = Value::DateTime(DateTime::now_utc());
    let mut directory = Object::new();
    directory.insert("id", Value::String(id.clone()));
    directory.insert("type", Value::String(DIRECTORY_CLASS_ID.to_string()));
    insert_canonical_field(
        &mut directory,
        "title",
        ATTR_TITLE,
        Value::String(title.to_string()),
    );
    insert_canonical_field(&mut directory, "created_at", ATTR_CREATED_AT, now.clone());
    insert_canonical_field(&mut directory, "updated_at", ATTR_UPDATED_AT, now);

    let mut operations = vec![Value::Object(batch_upsert_operation(
        ENTITIES_COLLECTION.to_string(),
        id.clone(),
        directory,
    ))];
    if let Some(parent_id) = parent {
        let order = next_directory_order(client.clone(), scope_id.clone(), &parent_id).await?;
        operations.push(Value::Object(batch_upsert_operation(
            ENTITIES_COLLECTION.to_string(),
            directory_node_id(&parent_id, &id),
            directory_node_object(&parent_id, &id, order),
        )));
    }
    run_batch_operations(client, scope_id, operations).await?;
    Ok(id)
}

pub async fn add_items_to_directory(
    client: semantic_rpc::RpcClient,
    scope_id: Option<String>,
    target_directory_id: String,
    item_ids: Vec<String>,
) -> std::result::Result<(), String> {
    add_items_to_directory_inner(client, scope_id, target_directory_id, item_ids).await
}

/// Creates an entity and its directory link in one ordered database batch.
pub async fn create_entity_in_directory(
    client: semantic_rpc::RpcClient,
    scope_id: Option<String>,
    target_directory_id: String,
    entity_id: String,
    entity: Object,
) -> std::result::Result<(), String> {
    validate_directory(client.clone(), scope_id.clone(), &target_directory_id).await?;
    let order =
        next_directory_order(client.clone(), scope_id.clone(), &target_directory_id).await?;
    run_batch_operations(
        client,
        scope_id,
        create_entity_in_directory_operations(&target_directory_id, &entity_id, entity, order),
    )
    .await
}

fn create_entity_in_directory_operations(
    target_directory_id: &str,
    entity_id: &str,
    entity: Object,
    order: u64,
) -> Vec<Value> {
    vec![
        Value::Object(batch_upsert_operation(
            ENTITIES_COLLECTION.to_string(),
            entity_id.to_string(),
            entity,
        )),
        Value::Object(batch_upsert_operation(
            ENTITIES_COLLECTION.to_string(),
            directory_node_id(target_directory_id, entity_id),
            directory_node_object(target_directory_id, entity_id, order),
        )),
    ]
}

pub(super) async fn copy_items_to_directory(
    client: semantic_rpc::RpcClient,
    scope_id: Option<String>,
    target_directory_id: String,
    item_ids: Vec<String>,
) -> std::result::Result<(), String> {
    add_items_to_directory_inner(client, scope_id, target_directory_id, item_ids).await
}

pub(super) async fn cut_items_to_directory(
    client: semantic_rpc::RpcClient,
    scope_id: Option<String>,
    source_directory_id: Option<String>,
    target_directory_id: String,
    item_ids: Vec<String>,
) -> std::result::Result<(), String> {
    if source_directory_id.as_deref() == Some(target_directory_id.as_str()) {
        return Ok(());
    }
    validate_directory(client.clone(), scope_id.clone(), &target_directory_id).await?;
    let item_ids = dedupe_ids(item_ids);
    if item_ids.is_empty() {
        return Ok(());
    }
    validate_addable_items(
        client.clone(),
        scope_id.clone(),
        &target_directory_id,
        &item_ids,
    )
    .await?;

    let existing_target_links = load_child_links(
        client.clone(),
        scope_id.clone(),
        &target_directory_id,
        &item_ids,
    )
    .await?;
    let linked_target_ids = existing_target_links
        .into_iter()
        .filter_map(|link| link.child_id)
        .collect::<BTreeSet<_>>();

    let mut order =
        next_directory_order(client.clone(), scope_id.clone(), &target_directory_id).await?;
    let mut operations = Vec::new();
    if let Some(source_id) = source_directory_id.as_deref() {
        let source_links =
            load_child_links(client.clone(), scope_id.clone(), source_id, &item_ids).await?;
        operations.extend(source_links.into_iter().map(|link| {
            Value::Object(batch_delete_operation(
                ENTITIES_COLLECTION.to_string(),
                link.node_id,
            ))
        }));
    }
    for item_id in item_ids {
        if linked_target_ids.contains(&item_id) {
            continue;
        }
        operations.push(Value::Object(batch_upsert_operation(
            ENTITIES_COLLECTION.to_string(),
            directory_node_id(&target_directory_id, &item_id),
            directory_node_object(&target_directory_id, &item_id, order),
        )));
        order = order.saturating_add(1);
    }
    run_batch_operations(client, scope_id, operations).await
}

pub(super) async fn unlink_items_from_directory(
    client: semantic_rpc::RpcClient,
    scope_id: Option<String>,
    parent_id: String,
    item_ids: Vec<String>,
) -> std::result::Result<UnlinkOutcome, String> {
    unlink_items_from_directory_inner(client, scope_id, parent_id, item_ids, false).await
}

pub(super) async fn unlink_items_from_directory_confirmed(
    client: semantic_rpc::RpcClient,
    scope_id: Option<String>,
    parent_id: String,
    item_ids: Vec<String>,
) -> std::result::Result<UnlinkOutcome, String> {
    unlink_items_from_directory_inner(client, scope_id, parent_id, item_ids, true).await
}

pub(super) async fn hard_delete_items(
    client: semantic_rpc::RpcClient,
    scope_id: Option<String>,
    item_ids: Vec<String>,
) -> std::result::Result<(), String> {
    let item_ids = dedupe_ids(item_ids);
    if item_ids.is_empty() {
        return Ok(());
    }

    let mut operations = Vec::new();
    for item_id in &item_ids {
        let incoming = load_parent_links(client.clone(), scope_id.clone(), item_id).await?;
        operations.extend(incoming.into_iter().map(|link| {
            Value::Object(batch_delete_operation(
                ENTITIES_COLLECTION.to_string(),
                link.node_id,
            ))
        }));
        if let Some(entity) = load_entity(
            client.clone(),
            scope_id.clone(),
            ENTITIES_COLLECTION,
            item_id,
        )
        .await?
            && is_directory_object(&entity)
        {
            let outgoing =
                load_directory_outgoing_links(client.clone(), scope_id.clone(), item_id).await?;
            operations.extend(outgoing.into_iter().map(|link| {
                Value::Object(batch_delete_operation(
                    ENTITIES_COLLECTION.to_string(),
                    link.node_id,
                ))
            }));
        }
        operations.push(Value::Object(batch_delete_operation(
            ENTITIES_COLLECTION.to_string(),
            item_id.clone(),
        )));
    }
    run_batch_operations(client, scope_id, operations).await
}

#[allow(dead_code)]
pub(super) async fn delete_entities(
    client: semantic_rpc::RpcClient,
    scope_id: Option<String>,
    item_ids: Vec<String>,
) -> std::result::Result<(), String> {
    hard_delete_items(client, scope_id, item_ids).await
}

pub(super) async fn rename_directory_item(
    client: semantic_rpc::RpcClient,
    scope_id: Option<String>,
    item_id: String,
    title: String,
) -> std::result::Result<(), String> {
    let title = title.trim();
    if title.is_empty() {
        return Err("Title is required".to_string());
    }
    let Some(mut entity) = load_entity(
        client.clone(),
        scope_id.clone(),
        ENTITIES_COLLECTION,
        &item_id,
    )
    .await?
    else {
        return Err("Item not found".to_string());
    };
    let now = Value::DateTime(DateTime::now_utc());
    insert_canonical_field(
        &mut entity,
        "title",
        ATTR_TITLE,
        Value::String(title.to_string()),
    );
    insert_canonical_field(&mut entity, "updated_at", ATTR_UPDATED_AT, now);
    run_batch_operations(
        client,
        scope_id,
        vec![Value::Object(batch_upsert_operation(
            ENTITIES_COLLECTION.to_string(),
            item_id,
            entity,
        ))],
    )
    .await
}

pub(super) async fn load_addable_entity_options(
    client: semantic_rpc::RpcClient,
    scope_id: Option<String>,
    parent_id: String,
    search: String,
    limit: usize,
) -> std::result::Result<Vec<DirectoryBrowseItem>, String> {
    validate_directory(client.clone(), scope_id.clone(), &parent_id).await?;
    let rows = run_select_query(
        client,
        scope_id,
        addable_entities_query(&parent_id, &search, limit),
    )
    .await?;
    Ok(rows.into_iter().map(row_to_item).collect())
}

pub(super) async fn directory_parent_count(
    client: semantic_rpc::RpcClient,
    scope_id: Option<String>,
    child_id: &str,
) -> std::result::Result<usize, String> {
    let rows = run_select_query(client, scope_id, parent_count_query(child_id)).await?;
    Ok(rows
        .first()
        .and_then(|row| row.get("parent_count"))
        .and_then(value_as_usize)
        .unwrap_or(0))
}

#[allow(dead_code)]
pub(super) async fn load_child_ids_for_directory(
    client: semantic_rpc::RpcClient,
    scope_id: Option<String>,
    parent_id: &str,
) -> std::result::Result<BTreeSet<String>, String> {
    let rows = run_select_query(client, scope_id, child_ids_query(parent_id)).await?;
    Ok(rows
        .iter()
        .filter_map(|row| {
            row.get("directory_to")
                .or_else(|| row.get(ATTR_RELATION_TO))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .collect())
}

#[allow(dead_code)]
pub(super) async fn load_directory_links_for_parent(
    client: semantic_rpc::RpcClient,
    scope_id: Option<String>,
    parent_id: &str,
    child_ids: &[String],
) -> std::result::Result<Vec<DirectoryLink>, String> {
    load_child_links(client, scope_id, parent_id, child_ids).await
}

#[allow(dead_code)]
pub(super) async fn load_all_directory_links_for_items(
    client: semantic_rpc::RpcClient,
    scope_id: Option<String>,
    item_ids: &[String],
) -> std::result::Result<Vec<DirectoryLink>, String> {
    let mut links = Vec::new();
    for item_id in item_ids {
        links.extend(load_parent_links(client.clone(), scope_id.clone(), item_id).await?);
        if let Some(entity) = load_entity(
            client.clone(),
            scope_id.clone(),
            ENTITIES_COLLECTION,
            item_id,
        )
        .await?
            && is_directory_object(&entity)
        {
            links.extend(
                load_directory_outgoing_links(client.clone(), scope_id.clone(), item_id).await?,
            );
        }
    }
    Ok(links)
}

pub(super) async fn next_directory_order(
    client: semantic_rpc::RpcClient,
    scope_id: Option<String>,
    parent_id: &str,
) -> std::result::Result<u64, String> {
    let rows = run_select_query(client, scope_id, max_child_order_query(parent_id)).await?;
    Ok(rows
        .first()
        .and_then(|row| row.get("max_order"))
        .and_then(|value| value_as_u64(Some(value)))
        .map(|order| order.saturating_add(1))
        .unwrap_or(0))
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct DirectoryLink {
    pub node_id: String,
    pub parent_id: Option<String>,
    pub child_id: Option<String>,
    pub order: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
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

fn directory_link_from_row(row: &Object) -> Option<DirectoryLink> {
    Some(DirectoryLink {
        node_id: row.get("id").and_then(Value::as_str)?.to_string(),
        parent_id: row
            .get("directory_from")
            .or_else(|| row.get(ATTR_DIRECTORY_NODE_FROM))
            .and_then(Value::as_str)
            .map(str::to_string),
        child_id: row
            .get("directory_to")
            .or_else(|| row.get(ATTR_RELATION_TO))
            .or_else(|| row.get("to"))
            .and_then(Value::as_str)
            .map(str::to_string),
        order: value_as_u64(
            row.get("directory_order")
                .or_else(|| row.get("order"))
                .or_else(|| row.get(ATTR_DIRECTORY_NODE_ORDER)),
        ),
    })
}

async fn load_child_links(
    client: semantic_rpc::RpcClient,
    scope_id: Option<String>,
    parent_id: &str,
    child_ids: &[String],
) -> std::result::Result<Vec<DirectoryLink>, String> {
    let rows = run_select_query(client, scope_id, child_links_query(parent_id, child_ids)).await?;
    Ok(rows.iter().filter_map(directory_link_from_row).collect())
}

async fn load_parent_links(
    client: semantic_rpc::RpcClient,
    scope_id: Option<String>,
    child_id: &str,
) -> std::result::Result<Vec<DirectoryLink>, String> {
    let rows = run_select_query(client, scope_id, parent_links_query(child_id)).await?;
    Ok(rows.iter().filter_map(directory_link_from_row).collect())
}

async fn load_directory_outgoing_links(
    client: semantic_rpc::RpcClient,
    scope_id: Option<String>,
    directory_id: &str,
) -> std::result::Result<Vec<DirectoryLink>, String> {
    let rows = run_select_query(
        client,
        scope_id,
        directory_outgoing_links_query(directory_id),
    )
    .await?;
    Ok(rows.iter().filter_map(directory_link_from_row).collect())
}

async fn add_items_to_directory_inner(
    client: semantic_rpc::RpcClient,
    scope_id: Option<String>,
    target_directory_id: String,
    item_ids: Vec<String>,
) -> std::result::Result<(), String> {
    validate_directory(client.clone(), scope_id.clone(), &target_directory_id).await?;
    let item_ids = dedupe_ids(item_ids);
    if item_ids.is_empty() {
        return Ok(());
    }
    validate_addable_items(
        client.clone(),
        scope_id.clone(),
        &target_directory_id,
        &item_ids,
    )
    .await?;

    let existing_links = load_child_links(
        client.clone(),
        scope_id.clone(),
        &target_directory_id,
        &item_ids,
    )
    .await?;
    let existing_ids = existing_links
        .into_iter()
        .filter_map(|link| link.child_id)
        .collect::<BTreeSet<_>>();
    let mut order =
        next_directory_order(client.clone(), scope_id.clone(), &target_directory_id).await?;
    let mut operations = Vec::new();
    for item_id in item_ids {
        if existing_ids.contains(&item_id) {
            continue;
        }
        operations.push(Value::Object(batch_upsert_operation(
            ENTITIES_COLLECTION.to_string(),
            directory_node_id(&target_directory_id, &item_id),
            directory_node_object(&target_directory_id, &item_id, order),
        )));
        order = order.saturating_add(1);
    }
    run_batch_operations(client, scope_id, operations).await
}

async fn unlink_items_from_directory_inner(
    client: semantic_rpc::RpcClient,
    scope_id: Option<String>,
    parent_id: String,
    item_ids: Vec<String>,
    delete_orphaned_directories: bool,
) -> std::result::Result<UnlinkOutcome, String> {
    validate_directory(client.clone(), scope_id.clone(), &parent_id).await?;
    let item_ids = dedupe_ids(item_ids);
    if item_ids.is_empty() {
        return Ok(UnlinkOutcome {
            removed_item_ids: Vec::new(),
            orphaned_directory_ids: Vec::new(),
            requires_confirmation: false,
        });
    }

    let links = load_child_links(client.clone(), scope_id.clone(), &parent_id, &item_ids).await?;
    let linked_ids = links
        .iter()
        .filter_map(|link| link.child_id.clone())
        .collect::<BTreeSet<_>>();
    let mut orphaned_directory_ids = Vec::new();
    for item_id in &linked_ids {
        let Some(entity) = load_entity(
            client.clone(),
            scope_id.clone(),
            ENTITIES_COLLECTION,
            item_id,
        )
        .await?
        else {
            continue;
        };
        if is_directory_object(&entity)
            && directory_parent_count(client.clone(), scope_id.clone(), item_id).await? <= 1
        {
            orphaned_directory_ids.push(item_id.clone());
        }
    }
    if !orphaned_directory_ids.is_empty() && !delete_orphaned_directories {
        return Ok(UnlinkOutcome {
            removed_item_ids: linked_ids.into_iter().collect(),
            orphaned_directory_ids,
            requires_confirmation: true,
        });
    }

    let orphaned_set = orphaned_directory_ids
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut operations = links
        .into_iter()
        .map(|link| {
            Value::Object(batch_delete_operation(
                ENTITIES_COLLECTION.to_string(),
                link.node_id,
            ))
        })
        .collect::<Vec<_>>();
    for directory_id in &orphaned_directory_ids {
        let outgoing =
            load_directory_outgoing_links(client.clone(), scope_id.clone(), directory_id).await?;
        operations.extend(outgoing.into_iter().map(|link| {
            Value::Object(batch_delete_operation(
                ENTITIES_COLLECTION.to_string(),
                link.node_id,
            ))
        }));
        operations.push(Value::Object(batch_delete_operation(
            ENTITIES_COLLECTION.to_string(),
            directory_id.clone(),
        )));
    }

    run_batch_operations(client, scope_id, operations).await?;
    Ok(UnlinkOutcome {
        removed_item_ids: linked_ids.into_iter().collect(),
        orphaned_directory_ids: orphaned_set.into_iter().collect(),
        requires_confirmation: false,
    })
}

async fn target_has_ancestor(
    client: semantic_rpc::RpcClient,
    scope_id: Option<String>,
    target_id: &str,
    ancestor_id: &str,
) -> std::result::Result<bool, String> {
    let mut seen = BTreeSet::new();
    let mut pending = vec![target_id.to_string()];
    for _ in 0..64 {
        let Some(current_id) = pending.pop() else {
            return Ok(false);
        };
        if !seen.insert(current_id.clone()) {
            continue;
        }
        for parent in load_parent_links(client.clone(), scope_id.clone(), &current_id).await? {
            let Some(parent_id) = parent.parent_id else {
                continue;
            };
            if parent_id == ancestor_id {
                return Ok(true);
            }
            pending.push(parent_id);
        }
    }
    Err("Directory ancestry is too deep or cyclic".to_string())
}

async fn validate_directory(
    client: semantic_rpc::RpcClient,
    scope_id: Option<String>,
    directory_id: &str,
) -> std::result::Result<Object, String> {
    let Some(directory) = load_entity(client, scope_id, ENTITIES_COLLECTION, directory_id).await?
    else {
        return Err("Target directory not found".to_string());
    };
    if !is_directory_object(&directory) {
        return Err("Target must be a directory".to_string());
    }
    Ok(directory)
}

async fn validate_addable_items(
    client: semantic_rpc::RpcClient,
    scope_id: Option<String>,
    target_directory_id: &str,
    item_ids: &[String],
) -> std::result::Result<(), String> {
    for item_id in item_ids {
        if item_id == target_directory_id {
            return Err("Cannot add a directory to itself".to_string());
        }
        let Some(item) = load_entity(
            client.clone(),
            scope_id.clone(),
            ENTITIES_COLLECTION,
            item_id,
        )
        .await?
        else {
            return Err(format!("Item not found: {item_id}"));
        };
        if is_directory_object(&item)
            && target_has_ancestor(
                client.clone(),
                scope_id.clone(),
                target_directory_id,
                item_id,
            )
            .await?
        {
            return Err("Cannot add a directory to one of its descendants".to_string());
        }
    }
    Ok(())
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

pub(super) fn hex_id_part(value: &str) -> String {
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

async fn run_batch_operations(
    client: semantic_rpc::RpcClient,
    scope_id: Option<String>,
    operations: Vec<Value>,
) -> std::result::Result<(), String> {
    if operations.is_empty() {
        return Ok(());
    }
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

fn dedupe_ids(item_ids: Vec<String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    item_ids
        .into_iter()
        .filter(|item_id| seen.insert(item_id.clone()))
        .collect()
}

fn unix_time_millis() -> u128 {
    (time::UtcDateTime::now().unix_timestamp_nanos().max(0) / 1_000_000) as u128
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
        created_at: object_string(&row, ATTR_CREATED_AT).map(str::to_string),
        updated_at: object_string(&row, ATTR_UPDATED_AT).map(str::to_string),
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
    object_string(object, ATTR_TITLE)
        .unwrap_or(fallback)
        .to_string()
}

fn object_string<'a>(object: &'a Object, key: &str) -> Option<&'a str> {
    object.get(key).and_then(Value::as_str)
}

fn insert_canonical_field(object: &mut Object, alias: &str, canonical: &str, value: Value) {
    object.remove(alias);
    object.insert(canonical, value);
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

fn value_as_usize(value: &Value) -> Option<usize> {
    match value {
        Value::U64(value) => (*value).try_into().ok(),
        Value::U32(value) => (*value).try_into().ok(),
        Value::U16(value) => Some(usize::from(*value)),
        Value::U8(value) => Some(usize::from(*value)),
        Value::I64(value) => (*value).try_into().ok(),
        Value::I32(value) => (*value).try_into().ok(),
        Value::I16(value) => (*value).try_into().ok(),
        Value::I8(value) => (*value).try_into().ok(),
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

    #[test]
    fn row_to_item_reads_canonical_directory_fields_only() {
        let mut object = Object::new();
        object.insert("id", Value::String("item-1".to_string()));
        object.insert("title", Value::String("Plain Title".to_string()));
        object.insert(ATTR_TITLE, Value::String("Canonical Title".to_string()));
        object.insert("created_at", Value::String("plain-created".to_string()));
        object.insert(
            ATTR_CREATED_AT,
            Value::String("canonical-created".to_string()),
        );

        let item = row_to_item(object);

        assert_eq!(item.title, "Canonical Title");
        assert_eq!(item.created_at.as_deref(), Some("canonical-created"));

        let mut plain_only = Object::new();
        plain_only.insert("id", Value::String("item-2".to_string()));
        plain_only.insert("title", Value::String("Plain Title".to_string()));
        plain_only.insert("created_at", Value::String("plain-created".to_string()));

        let item = row_to_item(plain_only);

        assert_eq!(item.title, "item-2");
        assert_eq!(item.created_at, None);
    }

    #[test]
    fn create_entity_directory_batch_puts_entity_before_link() {
        let mut entity = Object::new();
        entity.insert("id", Value::String("entity-1".to_string()));

        let operations =
            create_entity_in_directory_operations("directory-1", "entity-1", entity, 7);

        assert_eq!(operations.len(), 2);
        let Value::Object(entity_operation) = &operations[0] else {
            panic!("entity operation must be an object");
        };
        assert_eq!(
            entity_operation.get("collection"),
            Some(&Value::String(ENTITIES_COLLECTION.to_string()))
        );
        assert_eq!(
            entity_operation.get("id"),
            Some(&Value::String("entity-1".to_string()))
        );

        let Value::Object(link_operation) = &operations[1] else {
            panic!("link operation must be an object");
        };
        let Some(Value::Object(link)) = link_operation.get("object") else {
            panic!("link operation must contain the directory node");
        };
        assert_eq!(
            link.get(ATTR_DIRECTORY_NODE_FROM),
            Some(&Value::String("directory-1".to_string()))
        );
        assert_eq!(
            link.get(ATTR_RELATION_TO),
            Some(&Value::String("entity-1".to_string()))
        );
        assert_eq!(link.get(ATTR_DIRECTORY_NODE_ORDER), Some(&Value::U64(7)));
    }
}
