use super::*;
use crate::schema::labels::*;
use semantic_data::{
    query::Batch,
    value::{Object, Value},
};
use semantic_db_core::{Db, QueryResult, embedded::EmbeddedBackend};
use semantic_rpc_core::RpcError;

struct Store(std::sync::Arc<Db>);
impl LabelStore for Store {
    async fn select(&self, sql: String) -> Result<Vec<Object>, RpcError> {
        match self.0.query(sql).await.map_err(db_error)? {
            QueryResult::Select(rows) => Ok(rows),
            _ => unreachable!(),
        }
    }
    async fn get(&self, collection: &str, id: &str) -> Result<Option<Object>, RpcError> {
        self.0
            .get(collection, id)
            .await
            .map(|row| row.map(|row| row.object))
            .map_err(db_error)
    }
    async fn commit(&self, batch: Batch) -> Result<(), RpcError> {
        self.0
            .execute_batch(batch)
            .await
            .map(|_| ())
            .map_err(db_error)
    }
}
fn db_error(error: semantic_db_core::DbError) -> RpcError {
    RpcError::internal(error.to_string())
}

fn entity_object() -> Object {
    let mut object = Object::new();
    object.insert("id", Value::String("entity".into()));
    object.insert(
        "type",
        Value::String(crate::schema::common::person::CLASS_ID.into()),
    );
    object
}

async fn setup() -> Store {
    let db = Db::new(EmbeddedBackend::new(semantic_db_kv::open_memory().unwrap()));
    db.upsert_package(crate::package()).await.unwrap();
    db.insert("entities", "entity", entity_object())
        .await
        .unwrap();
    let store = Store(std::sync::Arc::new(db));
    let mut status = Label::new_group("status", "Status");
    status.selection_mode = SelectionMode::Exclusive;
    save_label(&store, status).await.unwrap();
    for id in ["todo", "done"] {
        let mut label = Label::new(id, id);
        label.parent_id = Some("status".into());
        save_label(&store, label).await.unwrap();
    }
    save_label(&store, Label::new("topic", "Topic"))
        .await
        .unwrap();
    store
}
fn ids(labels: Vec<Label>) -> Vec<String> {
    labels.into_iter().map(|label| label.id).collect()
}
fn strings(ids: &[&str]) -> Vec<String> {
    ids.iter().map(|id| (*id).into()).collect()
}

#[tokio::test]
async fn assignment_exclusivity_idempotence_and_clear() {
    let store = setup().await;
    assert_eq!(
        ids(add_labels(
            &store,
            "entities",
            "entity",
            &strings(&["todo", "topic", "todo"])
        )
        .await
        .unwrap()),
        strings(&["todo", "topic"])
    );
    add_labels(&store, "entities", "entity", &strings(&["todo"]))
        .await
        .unwrap();
    let links = store
        .select(format!(
            "SELECT * FROM entities WHERE type = '{RELATION_ID}'"
        ))
        .await
        .unwrap();
    assert_eq!(links.len(), 2);
    assert_eq!(
        ids(
            add_labels(&store, "entities", "entity", &strings(&["done"]))
                .await
                .unwrap()
        ),
        strings(&["done", "topic"])
    );
    assert_eq!(
        replace_labels(&store, "entities", "entity", &strings(&["done", "todo"]))
            .await
            .unwrap_err()
            .code,
        "label_conflict"
    );
    assert_eq!(
        ids(labels_for_entity(&store, "entities", "entity")
            .await
            .unwrap()),
        strings(&["done", "topic"])
    );
    assert_eq!(
        ids(
            remove_labels(&store, "entities", "entity", &strings(&["done", "missing"]))
                .await
                .unwrap()
        ),
        strings(&["topic"])
    );
    assert!(
        replace_labels(&store, "entities", "entity", &[])
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        labels_for_entity(&store, "entities", "entity")
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn hierarchy_validation_and_delete_cleanup() {
    let store = setup().await;
    let mut status = list_labels(&store)
        .await
        .unwrap()
        .into_iter()
        .find(|l| l.id == "status")
        .unwrap();
    let created = status.created_at;
    status.parent_id = Some("todo".into());
    assert_eq!(
        save_label(&store, status.clone()).await.unwrap_err().code,
        "label_cycle"
    );
    status.parent_id = Some("missing".into());
    assert_eq!(
        save_label(&store, status.clone()).await.unwrap_err().code,
        "label_not_found"
    );
    status.parent_id = None;
    status.name = " Workflow ".into();
    let saved = save_label(&store, status).await.unwrap();
    assert_eq!(saved.name, "Workflow");
    assert_eq!(saved.created_at, created);
    assert!(saved.updated_at >= created);
    assert_eq!(
        delete_label(&store, "status").await.unwrap_err().code,
        "label_has_children"
    );
    add_labels(&store, "entities", "entity", &strings(&["todo", "topic"]))
        .await
        .unwrap();
    delete_label(&store, "todo").await.unwrap();
    assert_eq!(
        ids(labels_for_entity(&store, "entities", "entity")
            .await
            .unwrap()),
        strings(&["topic"])
    );
    assert!(store.get("entities", "entity").await.unwrap().is_some());
    let links = store
        .select(format!(
            "SELECT * FROM entities WHERE type = '{RELATION_ID}'"
        ))
        .await
        .unwrap();
    assert_eq!(links.len(), 1);
}

#[tokio::test]
async fn mode_changes_and_reparenting_preserve_existing_selections() {
    let store = setup().await;
    let mut group = Label::new_group("group", "Group");
    save_label(&store, group.clone()).await.unwrap();
    for id in ["a", "b"] {
        let mut label = Label::new(id, id);
        label.parent_id = Some("group".into());
        save_label(&store, label).await.unwrap();
    }
    add_labels(&store, "entities", "entity", &strings(&["a", "b", "todo"]))
        .await
        .unwrap();
    group.selection_mode = SelectionMode::Exclusive;
    assert_eq!(
        save_label(&store, group.clone()).await.unwrap_err().code,
        "label_conflict"
    );
    let mut a = Label::new("a", "A");
    a.parent_id = Some("status".into());
    assert_eq!(
        save_label(&store, a).await.unwrap_err().code,
        "label_conflict"
    );
    remove_labels(&store, "entities", "entity", &strings(&["b"]))
        .await
        .unwrap();
    save_label(&store, group).await.unwrap();
}

#[tokio::test]
async fn separate_collections_missing_entities_and_invalid_inputs() {
    let store = setup().await;
    store
        .0
        .create_collection(
            "other",
            semantic_db_core::catalog::CollectionKind::Polymorphic,
        )
        .await
        .unwrap();
    store
        .0
        .insert("other", "entity", entity_object())
        .await
        .unwrap();
    add_labels(&store, "entities", "entity", &strings(&["todo"]))
        .await
        .unwrap();
    add_labels(&store, "other", "entity", &strings(&["done"]))
        .await
        .unwrap();
    assert_eq!(
        ids(labels_for_entity(&store, "other", "entity").await.unwrap()),
        strings(&["done"])
    );
    assert_eq!(
        ids(labels_for_entity(&store, "entities", "entity")
            .await
            .unwrap()),
        strings(&["todo"])
    );
    assert_eq!(
        add_labels(&store, "entities", "missing", &strings(&["todo"]))
            .await
            .unwrap_err()
            .code,
        "entity_not_found"
    );
    assert_eq!(
        add_labels(&store, "entities", "entity", &strings(&["missing"]))
            .await
            .unwrap_err()
            .code,
        "label_not_found"
    );
    let mut invalid = Label::new("invalid", " ");
    assert!(save_label(&store, invalid.clone()).await.is_err());
    invalid.name = "Invalid".into();
    invalid.color = Some("red; display:none".into());
    assert!(save_label(&store, invalid).await.is_err());
    assert_eq!(
        save_label(&store, Label::new("entity", "Collision"))
            .await
            .unwrap_err()
            .code,
        "label_id_conflict"
    );
}

#[tokio::test]
async fn concurrent_adds_do_not_lose_updates_or_break_exclusivity() {
    let store = setup().await;
    let todo = strings(&["todo"]);
    let done = strings(&["done"]);
    let topic = strings(&["topic"]);
    let (a, b, c) = futures::join!(
        add_labels(&store, "entities", "entity", &todo),
        add_labels(&store, "entities", "entity", &done),
        add_labels(&store, "entities", "entity", &topic)
    );
    a.unwrap();
    b.unwrap();
    c.unwrap();
    let assigned = labels_for_entity(&store, "entities", "entity")
        .await
        .unwrap();
    assert_eq!(assigned.len(), 2);
    assert!(assigned.iter().any(|label| label.id == "topic"));
}

#[test]
fn model_round_trip_and_selection_paths() {
    let mut parent = Label::new_group("p", "Status");
    parent.selection_mode = SelectionMode::Exclusive;
    let mut a = Label::new("a", "First");
    a.parent_id = Some("p".into());
    a.color = Some("#abcdef".into());
    let mut b = Label::new("b", "Second");
    b.parent_id = Some("p".into());
    assert_eq!(Label::from_object(&a.to_object()).unwrap(), a);
    let catalog = LabelCatalog::new([parent, a, b]);
    assert_eq!(catalog.path("a"), "Status / First");
    let mut selected = std::collections::BTreeSet::new();
    assert_eq!(
        catalog.select(&mut selected, "p").unwrap_err().code,
        "label_group_not_assignable"
    );
    catalog.select(&mut selected, "a").unwrap();
    catalog.select(&mut selected, "b").unwrap();
    assert_eq!(selected.into_iter().collect::<Vec<_>>(), strings(&["b"]));
    assert!(decode_labels(Value::String("bad".into())).is_err());
}

#[tokio::test]
async fn groups_are_not_assignable_and_conversion_preserves_memberships() {
    let store = setup().await;
    assert_eq!(
        add_labels(&store, "entities", "entity", &strings(&["status"]))
            .await
            .unwrap_err()
            .code,
        "label_group_not_assignable"
    );
    add_labels(&store, "entities", "entity", &strings(&["topic"]))
        .await
        .unwrap();
    let mut topic = list_labels(&store)
        .await
        .unwrap()
        .into_iter()
        .find(|label| label.id == "topic")
        .unwrap();
    topic.kind = LabelKind::Group;
    assert_eq!(
        save_label(&store, topic.clone()).await.unwrap_err().code,
        "label_has_assignments"
    );
    assert_eq!(
        ids(labels_for_entity(&store, "entities", "entity")
            .await
            .unwrap()),
        strings(&["topic"])
    );
    remove_labels(&store, "entities", "entity", &strings(&["topic"]))
        .await
        .unwrap();
    let group = save_label(&store, topic).await.unwrap();
    assert!(group.is_group());
    assert_eq!(Label::from_object(&group.to_object()).unwrap(), group);
    assert_eq!(
        replace_labels(&store, "entities", "entity", &strings(&["topic"]))
            .await
            .unwrap_err()
            .code,
        "label_group_not_assignable"
    );
}

#[tokio::test]
async fn optional_metadata_can_be_omitted_and_cleared_for_labels_and_groups() {
    let store = setup().await;
    for mut label in [
        Label::new("plain", "Plain"),
        Label::new_group("plain-group", "Plain group"),
    ] {
        label = save_label(&store, label).await.unwrap();
        assert_eq!(label.description, None);
        assert_eq!(label.color, None);
        label.description = Some("Description".into());
        label.color = Some("#aabbcc".into());
        label = save_label(&store, label).await.unwrap();
        label.description = Some("  ".into());
        label.color = None;
        let label = save_label(&store, label).await.unwrap();
        let object = store.get("entities", &label.id).await.unwrap().unwrap();
        assert!(!object.contains_key(ATTR_DESCRIPTION));
        assert!(!object.contains_key(ATTR_COLOR));
        assert_eq!(Label::from_object(&object).unwrap(), label);
    }
}

#[tokio::test]
async fn migrated_group_assignments_remain_visible_and_removable() {
    let store = setup().await;
    save_label(&store, Label::new_group("another-group", "Another group"))
        .await
        .unwrap();
    // Represents a membership deliberately retained by the group migration.
    for (link_id, target) in [
        ("legacy-group-link", "status"),
        ("other-legacy-group-link", "another-group"),
    ] {
        let mut link = Object::new();
        for (key, value) in [
            ("id", link_id),
            ("type", RELATION_ID),
            (ATTR_RELATION, RELATION_ID),
            (ATTR_FROM, "entity"),
            (ATTR_TO, target),
            (ATTR_ENTITY_COLLECTION, "entities"),
        ] {
            link.insert(key, Value::String(value.into()));
        }
        store.0.insert("entities", link_id, link).await.unwrap();
    }
    let assigned = labels_for_entity(&store, "entities", "entity")
        .await
        .unwrap();
    assert_eq!(assigned.len(), 2);
    assert!(assigned.iter().all(Label::is_group));
    assert_eq!(
        replace_labels(&store, "entities", "entity", &strings(&["status"]))
            .await
            .unwrap_err()
            .code,
        "label_group_not_assignable"
    );
    assert_eq!(
        ids(
            remove_labels(&store, "entities", "entity", &strings(&["status"]))
                .await
                .unwrap()
        ),
        strings(&["another-group"])
    );
    assert!(
        remove_labels(&store, "entities", "entity", &strings(&["another-group"]))
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn label_commands_route_scope_before_reading_or_writing() {
    use semantic_rpc_core::RpcCommand;
    struct Context {
        default: Store,
        other: Store,
    }
    impl LabelContext for Context {
        type Store = Store;
        async fn label_store(&self, scope: Option<String>) -> Result<Store, RpcError> {
            match scope.as_deref() {
                None => Ok(Store(self.default.0.clone())),
                Some("other") => Ok(Store(self.other.0.clone())),
                _ => Err(RpcError::new("scope_denied", "Unknown scope")),
            }
        }
    }
    let ctx = Context {
        default: setup().await,
        other: setup().await,
    };
    let mut payload = Object::new();
    payload.insert("id", Value::String("entity".into()));
    payload.insert(
        "label_ids",
        Value::List(vec![Value::String("topic".into())]),
    );
    payload.insert("scope_id", Value::String("other".into()));
    AddEntityLabels
        .call(&ctx, Value::Object(payload.clone()))
        .await
        .unwrap();
    assert!(
        labels_for_entity(&ctx.default, "entities", "entity")
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        ids(decode_labels(
            LoadEntityLabels
                .call(&ctx, Value::Object(payload.clone()))
                .await
                .unwrap()
        )
        .unwrap()),
        strings(&["topic"])
    );
    payload.insert("scope_id", Value::String("denied".into()));
    assert_eq!(
        ReplaceEntityLabels
            .call(&ctx, Value::Object(payload))
            .await
            .unwrap_err()
            .code,
        "scope_denied"
    );
    assert_eq!(
        ids(labels_for_entity(&ctx.other, "entities", "entity")
            .await
            .unwrap()),
        strings(&["topic"])
    );
}

#[tokio::test]
async fn deleting_a_labeled_label_also_cleans_its_outgoing_assignments() {
    let store = setup().await;
    add_labels(&store, "entities", "topic", &strings(&["todo"]))
        .await
        .unwrap();
    delete_label(&store, "topic").await.unwrap();
    let links = store
        .select(format!(
            "SELECT * FROM entities WHERE type = '{RELATION_ID}'"
        ))
        .await
        .unwrap();
    assert!(links.is_empty());
}

#[tokio::test]
async fn membership_id_collision_cannot_overwrite_an_entity() {
    let store = setup().await;
    let id = "semantic:entity_label:8:entities6:entity4:todo";
    let mut object = entity_object();
    object.insert("id", Value::String(id.into()));
    store
        .0
        .insert("entities", id, object.clone())
        .await
        .unwrap();
    assert_eq!(
        add_labels(&store, "entities", "entity", &strings(&["todo"]))
            .await
            .unwrap_err()
            .code,
        "label_link_conflict"
    );
    assert_eq!(
        store
            .get("entities", id)
            .await
            .unwrap()
            .unwrap()
            .get("type"),
        object.get("type")
    );
    assert!(
        labels_for_entity(&store, "entities", "entity")
            .await
            .unwrap()
            .is_empty()
    );
}
