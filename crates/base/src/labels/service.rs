use super::{Label, LabelCatalog, model::required_string};
use crate::{
    directory_query::{sql_ident, sql_string},
    schema::labels::*,
};
use futures::lock::Mutex;
use semantic_data::{
    builtin::DEFAULT_COLLECTION,
    query::{Batch, BatchOperation},
    value::{DateTime, Object, Value},
};
use semantic_rpc_core::RpcError;
use std::{collections::BTreeSet, future::Future};

/// Minimal database interface for label helpers. `commit` must apply the batch atomically.
/// `select` returns complete objects, including their `id` and canonical attribute IDs.
/// Use the same database/scope for all calls on a store instance.
pub trait LabelStore: Send + Sync {
    fn select(&self, sql: String) -> impl Future<Output = Result<Vec<Object>, RpcError>> + Send;
    fn get(
        &self,
        collection: &str,
        id: &str,
    ) -> impl Future<Output = Result<Option<Object>, RpcError>> + Send;
    fn commit(&self, batch: Batch) -> impl Future<Output = Result<(), RpcError>> + Send;
}

// A shared async gate covers reads, validation, and commit, including schema edits that
// affect selections. The database API exposes atomic batches, not read/write transactions.
static LABEL_WRITES: Mutex<()> = Mutex::new(());

pub async fn list_labels(store: &impl LabelStore) -> Result<Vec<Label>, RpcError> {
    let rows = store
        .select(format!(
            "SELECT * FROM {} WHERE type = {} OR type = {} FORMAT QUALIFIED",
            sql_ident(DEFAULT_COLLECTION),
            sql_string(CLASS_ID),
            sql_string(GROUP_CLASS_ID)
        ))
        .await?;
    let mut labels = rows
        .iter()
        .map(Label::from_object)
        .collect::<Result<Vec<_>, _>>()?;
    labels.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then(a.id.cmp(&b.id))
    });
    Ok(labels)
}

async fn links(
    store: &impl LabelStore,
    entity: Option<(&str, &str)>,
) -> Result<Vec<Object>, RpcError> {
    let mut sql = format!(
        "SELECT * FROM {} WHERE {} = {}",
        sql_ident(DEFAULT_COLLECTION),
        sql_ident(ATTR_RELATION),
        sql_string(RELATION_ID)
    );
    if let Some((collection, id)) = entity {
        sql.push_str(&format!(
            " AND {} = {} AND {} = {}",
            sql_ident(ATTR_ENTITY_COLLECTION),
            sql_string(collection),
            sql_ident(ATTR_FROM),
            sql_string(id)
        ));
    }
    sql.push_str(" FORMAT QUALIFIED");
    store.select(sql).await
}

async fn require_entity(
    store: &impl LabelStore,
    collection: &str,
    id: &str,
) -> Result<(), RpcError> {
    if store.get(collection, id).await?.is_none() {
        return Err(RpcError::new(
            "entity_not_found",
            "The entity no longer exists",
        ));
    }
    Ok(())
}

pub async fn labels_for_entity(
    store: &impl LabelStore,
    collection: &str,
    id: &str,
) -> Result<Vec<Label>, RpcError> {
    let _guard = LABEL_WRITES.lock().await;
    require_entity(store, collection, id).await?;
    let assigned = link_targets(&links(store, Some((collection, id))).await?)?;
    Ok(list_labels(store)
        .await?
        .into_iter()
        .filter(|label| assigned.contains(&label.id))
        .collect())
}

fn link_targets(links: &[Object]) -> Result<BTreeSet<String>, RpcError> {
    links
        .iter()
        .map(|link| required_string(link, ATTR_TO))
        .collect()
}

#[derive(Clone, Copy)]
enum Change {
    Add,
    Remove,
    Replace,
}

pub async fn add_labels(
    store: &impl LabelStore,
    collection: &str,
    id: &str,
    label_ids: &[String],
) -> Result<Vec<Label>, RpcError> {
    change_labels(store, collection, id, label_ids, Change::Add).await
}

pub async fn remove_labels(
    store: &impl LabelStore,
    collection: &str,
    id: &str,
    label_ids: &[String],
) -> Result<Vec<Label>, RpcError> {
    change_labels(store, collection, id, label_ids, Change::Remove).await
}

/// Replace the complete selection. An empty list clears all labels.
pub async fn replace_labels(
    store: &impl LabelStore,
    collection: &str,
    id: &str,
    label_ids: &[String],
) -> Result<Vec<Label>, RpcError> {
    change_labels(store, collection, id, label_ids, Change::Replace).await
}

async fn change_labels(
    store: &impl LabelStore,
    collection: &str,
    id: &str,
    ids: &[String],
    change: Change,
) -> Result<Vec<Label>, RpcError> {
    let _guard = LABEL_WRITES.lock().await;
    require_entity(store, collection, id).await?;
    let catalog = LabelCatalog::new(list_labels(store).await?);
    let existing = links(store, Some((collection, id))).await?;
    let requested: BTreeSet<_> = ids.iter().cloned().collect();
    // An ambiguous multi-add is rejected; selecting one sibling replaces the old one.
    if !matches!(change, Change::Remove) {
        catalog.validate_selection(&requested)?;
    }
    let mut selected = link_targets(&existing)?;
    match change {
        Change::Replace => selected = requested,
        Change::Remove => selected.retain(|id| !requested.contains(id)),
        Change::Add => {
            for id in requested {
                catalog.select(&mut selected, &id)?;
            }
        }
    }
    // Removal can only reduce conflicts, and must allow gradual cleanup of
    // legacy group assignments or stale memberships.
    if !matches!(change, Change::Remove) {
        catalog.validate_selection(&selected)?;
    }
    let mut batch = Batch::new();
    // Preserve existing memberships and normalize any historical duplicate links.
    let mut retained = BTreeSet::new();
    for link in existing {
        let target = required_string(&link, ATTR_TO)?;
        if !selected.contains(&target) || !retained.insert(target) {
            batch
                .operations
                .push(delete_op(required_string(&link, "id")?));
        }
    }
    for label_id in selected.difference(&retained) {
        let link_id = membership_id(collection, id, label_id);
        if store.get(DEFAULT_COLLECTION, &link_id).await?.is_some() {
            return Err(RpcError::new(
                "label_link_conflict",
                "A label membership ID is already in use",
            ));
        }
        let mut object = Object::new();
        for (key, value) in [
            ("id", link_id.as_str()),
            ("type", RELATION_ID),
            (ATTR_RELATION, RELATION_ID),
            (ATTR_FROM, id),
            (ATTR_TO, label_id),
            (ATTR_ENTITY_COLLECTION, collection),
        ] {
            object.insert(key, Value::String(value.into()));
        }
        batch.operations.push(upsert_op(link_id, object));
    }
    if !batch.operations.is_empty() {
        store.commit(batch).await?;
    }
    Ok(catalog
        .labels
        .into_values()
        .filter(|label| selected.contains(&label.id))
        .collect())
}

fn membership_id(collection: &str, entity: &str, label: &str) -> String {
    // Length prefixes make arbitrary Unicode IDs and separators unambiguous.
    format!(
        "semantic:entity_label:{}:{collection}{}:{entity}{}:{label}",
        collection.len(),
        entity.len(),
        label.len()
    )
}

/// Create or edit a label, retaining its creation timestamp and unrelated attributes.
/// Changes that would invalidate existing selections are rejected without writing.
pub async fn save_label(store: &impl LabelStore, mut label: Label) -> Result<Label, RpcError> {
    let _guard = LABEL_WRITES.lock().await;
    label.name = label.name.trim().into();
    label.description = label.description.filter(|value| !value.trim().is_empty());
    let mut catalog = LabelCatalog::new(list_labels(store).await?);
    catalog.validate_label(&label)?;
    let previous = store.get(DEFAULT_COLLECTION, &label.id).await?;
    if previous.is_some() && !catalog.labels.contains_key(&label.id) {
        return Err(RpcError::new(
            "label_id_conflict",
            "This ID belongs to another entity",
        ));
    }
    label.created_at = catalog
        .labels
        .get(&label.id)
        .and_then(|old| old.created_at)
        .or(Some(DateTime::now_utc()));
    label.updated_at = Some(DateTime::now_utc());
    let hierarchy_changed = catalog.labels.get(&label.id).is_some_and(|old| {
        old.parent_id != label.parent_id
            || old.selection_mode != label.selection_mode
            || old.kind != label.kind
    });
    let becoming_group = label.is_group()
        && catalog
            .labels
            .get(&label.id)
            .is_some_and(|old| !old.is_group());
    catalog.labels.insert(label.id.clone(), label.clone());
    if hierarchy_changed {
        let mut assignments =
            std::collections::BTreeMap::<(String, String), BTreeSet<String>>::new();
        for link in links(store, None).await? {
            let target = required_string(&link, ATTR_TO)?;
            if becoming_group && target == label.id {
                return Err(RpcError::new(
                    "label_has_assignments",
                    "Remove this label's assignments before converting it to a group",
                ));
            }
            // Migration preserves historical group assignments. They do not
            // participate in sibling exclusivity and must remain removable.
            if catalog.labels.get(&target).is_some_and(Label::is_group) {
                continue;
            }
            assignments
                .entry((
                    required_string(&link, ATTR_ENTITY_COLLECTION)?,
                    required_string(&link, ATTR_FROM)?,
                ))
                .or_default()
                .insert(target);
        }
        for ids in assignments.values() {
            catalog.validate_selection(ids)?;
        }
    }
    let mut object = previous.unwrap_or_default();
    for key in [
        ATTR_DESCRIPTION,
        ATTR_PARENT,
        ATTR_COLOR,
        ATTR_SELECTION_MODE,
    ] {
        object.remove(key);
    }
    for (key, value) in label.to_object().iter() {
        object.insert(key.clone(), value.clone());
    }
    store
        .commit(Batch::new().with_op(upsert_op(label.id.clone(), object)))
        .await?;
    Ok(label)
}

/// Delete a leaf label and its memberships atomically. Reparent/delete children first.
pub async fn delete_label(store: &impl LabelStore, id: &str) -> Result<(), RpcError> {
    let _guard = LABEL_WRITES.lock().await;
    let labels = list_labels(store).await?;
    if !labels.iter().any(|label| label.id == id) {
        return Err(RpcError::new("label_not_found", "Label no longer exists"));
    }
    if labels
        .iter()
        .any(|label| label.parent_id.as_deref() == Some(id))
    {
        return Err(RpcError::new(
            "label_has_children",
            "Move or delete this label's children first",
        ));
    }
    let mut batch = Batch::new();
    for link in links(store, None).await? {
        if required_string(&link, ATTR_TO)? == id
            || (required_string(&link, ATTR_FROM)? == id
                && required_string(&link, ATTR_ENTITY_COLLECTION)? == DEFAULT_COLLECTION)
        {
            batch
                .operations
                .push(delete_op(required_string(&link, "id")?));
        }
    }
    batch.operations.push(delete_op(id.into()));
    store.commit(batch).await
}

fn delete_op(id: String) -> BatchOperation {
    BatchOperation::DeleteById {
        collection: DEFAULT_COLLECTION.into(),
        id,
    }
}
fn upsert_op(id: String, object: Object) -> BatchOperation {
    BatchOperation::Upsert {
        collection: DEFAULT_COLLECTION.into(),
        id,
        object,
    }
}
