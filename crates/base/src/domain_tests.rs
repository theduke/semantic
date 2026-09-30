use crate::{comments::*, content::MainContent, labels::LabelStore, tasks::*};
use semantic_data::{
    builtin::DEFAULT_COLLECTION,
    query::Batch,
    value::{FromValue, IntoValue, Object},
};
use semantic_db_core::{Db, QueryResult, embedded::EmbeddedBackend};
use semantic_rpc_core::RpcError;
struct Store(Db);
impl LabelStore for Store {
    async fn select(&self, sql: String) -> Result<Vec<Object>, RpcError> {
        match self
            .0
            .query(sql)
            .await
            .map_err(|e| RpcError::internal(e.to_string()))?
        {
            QueryResult::Select(rows) => Ok(rows),
            _ => Err(RpcError::internal("Expected rows")),
        }
    }
    async fn get(&self, collection: &str, id: &str) -> Result<Option<Object>, RpcError> {
        self.0
            .get(collection, id)
            .await
            .map(|r| r.map(|r| r.object))
            .map_err(|e| RpcError::internal(e.to_string()))
    }
    async fn commit(&self, batch: Batch) -> Result<(), RpcError> {
        self.0
            .execute_batch(batch)
            .await
            .map(|_| ())
            .map_err(|e| RpcError::internal(e.to_string()))
    }
}
async fn setup() -> Store {
    let db = Db::new(EmbeddedBackend::new(semantic_db_kv::open_memory().unwrap()));
    db.upsert_package(crate::package()).await.unwrap();
    db.upsert_package(crate::comments::package()).await.unwrap();
    db.upsert_package(crate::tasks::package()).await.unwrap();
    Store(db)
}
fn create(title: &str, parent: Option<String>) -> CreateTaskPayload {
    CreateTaskPayload {
        scope_id: None,
        title: title.into(),
        main_content: MainContent::note("**Description**"),
        status: TaskStatus::Todo,
        priority: TaskPriority::Medium,
        progress: None,
        due_date: None,
        parent,
    }
}
#[tokio::test]
async fn embedded_content_persistence_and_rpc() {
    let store = setup().await;
    let task = create_task(&store, create("  Hello  ", None))
        .await
        .unwrap();
    assert_eq!(task.title, "Hello");
    let loaded = get_task(&store, &task.id).await.unwrap();
    assert_eq!(loaded.main_content, task.main_content);
    assert_eq!(
        Task::from_value(loaded.clone().into_value()).unwrap(),
        loaded
    );
    let content = MainContent::from_value(loaded.main_content.to_value()).unwrap();
    assert_eq!(content.body(), "**Description**");
    let mut malformed = content.to_value();
    if let semantic_data::value::Value::Object(ref mut o) = malformed {
        o.insert("kind", "document".to_string().into_value());
    }
    assert!(MainContent::from_value(malformed).is_err());
}
#[tokio::test]
async fn hierarchy_progress_archive_patch() {
    let store = setup().await;
    assert!(create_task(&store, create(" ", None)).await.is_err());
    assert!(
        create_task(&store, create("Missing", Some("missing".into())))
            .await
            .is_err()
    );
    let parent = create_task(&store, create("Parent", None)).await.unwrap();
    let child = create_task(&store, create("Child", Some(parent.id.clone())))
        .await
        .unwrap();
    assert!(
        update_task(
            &store,
            UpdateTaskPayload {
                id: parent.id.clone(),
                parent: Some(child.id.clone()),
                ..Default::default()
            }
        )
        .await
        .is_err()
    );
    assert!(
        update_task(
            &store,
            UpdateTaskPayload {
                id: parent.id.clone(),
                parent: Some(parent.id.clone()),
                ..Default::default()
            }
        )
        .await
        .is_err()
    );
    let done = update_task(
        &store,
        UpdateTaskPayload {
            id: child.id.clone(),
            status: Some(TaskStatus::Done),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(done.progress, 100);
    assert!(
        update_task(
            &store,
            UpdateTaskPayload {
                id: child.id.clone(),
                progress: Some(42),
                ..Default::default()
            }
        )
        .await
        .is_err()
    );
    let reopened = update_task(
        &store,
        UpdateTaskPayload {
            id: child.id.clone(),
            status: Some(TaskStatus::Todo),
            clear_parent: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(reopened.progress, 0);
    assert!(reopened.parent.is_none());
    assert_eq!(reopened.created_at, child.created_at);
    archive_task(&store, &child.id, true).await.unwrap();
    assert_eq!(
        list_tasks(
            &store,
            TaskListPayload {
                limit: 50,
                ..Default::default()
            }
        )
        .await
        .unwrap()
        .total,
        1
    );
    assert_eq!(
        list_tasks(
            &store,
            TaskListPayload {
                limit: 50,
                include_archived: true,
                ..Default::default()
            }
        )
        .await
        .unwrap()
        .total,
        2
    );
}
#[tokio::test]
async fn generic_comments_ownership_threads_and_tombstone() {
    let store = setup().await;
    let task = create_task(&store, create("Subject", None)).await.unwrap();
    let target = EntityTarget {
        collection: DEFAULT_COLLECTION.into(),
        id: task.id.clone(),
    };
    let other = create_task(&store, create("Other", None)).await.unwrap();
    let root = create_comment(
        &store,
        CreateCommentPayload {
            scope_id: None,
            target: target.clone(),
            main_content: MainContent::note("Root"),
            parent: None,
        },
        "alice".into(),
    )
    .await
    .unwrap();
    assert!(
        create_comment(
            &store,
            CreateCommentPayload {
                scope_id: None,
                target: EntityTarget {
                    collection: DEFAULT_COLLECTION.into(),
                    id: other.id
                },
                main_content: MainContent::note("wrong"),
                parent: Some(root.id.clone())
            },
            "alice".into()
        )
        .await
        .is_err()
    );
    let reply = create_comment(
        &store,
        CreateCommentPayload {
            scope_id: None,
            target: target.clone(),
            main_content: MainContent::text("Reply"),
            parent: Some(root.id.clone()),
        },
        "bob".into(),
    )
    .await
    .unwrap();
    assert!(
        edit_comment(
            &store,
            &root.id,
            Some(MainContent::note("No")),
            "bob",
            false
        )
        .await
        .is_err()
    );
    let edited = edit_comment(
        &store,
        &root.id,
        Some(MainContent::note("Edited")),
        "alice",
        false,
    )
    .await
    .unwrap();
    assert_eq!(edited.main_content.body(), "Edited");
    let deleted = edit_comment(&store, &root.id, None, "alice", false)
        .await
        .unwrap();
    assert!(deleted.deleted);
    assert!(deleted.main_content.body().is_empty());
    edit_comment(&store, &root.id, None, "alice", false)
        .await
        .unwrap();
    let page = list_comments(
        &store,
        ListCommentsPayload {
            scope_id: None,
            target,
            offset: 0,
            limit: 50,
        },
    )
    .await
    .unwrap();
    assert_eq!(page.total, 2);
    assert_eq!(page.comments[1].id, reply.id);
    assert_eq!(page.comments[1].parent.as_deref(), Some(root.id.as_str()));
}

#[tokio::test]
async fn old_base_upgrade_and_package_replay() {
    let db = Db::new(EmbeddedBackend::new(semantic_db_kv::open_memory().unwrap()));
    let mut old = crate::package();
    old.migrations.pop();
    old.root
        .attributes
        .remove(crate::content::ATTR_MAIN_CONTENT);
    db.upsert_package(old).await.unwrap();
    db.upsert_package(crate::package()).await.unwrap();
    db.upsert_package(crate::comments::package()).await.unwrap();
    db.upsert_package(crate::tasks::package()).await.unwrap();
    db.upsert_package(crate::package()).await.unwrap();
    db.upsert_package(crate::comments::package()).await.unwrap();
    db.upsert_package(crate::tasks::package()).await.unwrap();
}
#[tokio::test]
async fn patches_preserve_unknown_fields_and_dates() {
    let store = setup().await;
    let mut input = create("Dates", None);
    input.due_date = Some(DueDate(semantic_data::value::Date::now_utc()));
    let task = create_task(&store, input).await.unwrap();
    let mut object = task.to_object();
    object.insert(
        semantic_data::attr::ATTR_DESCRIPTION,
        "untouched".to_owned().into_value(),
    );
    crate::domain_support::persist(&store, task.id.clone(), object)
        .await
        .unwrap();
    let updated = update_task(
        &store,
        UpdateTaskPayload {
            id: task.id.clone(),
            title: Some("Changed".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(updated.due_date, task.due_date);
    let object = store
        .get(DEFAULT_COLLECTION, &task.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        object
            .get(semantic_data::attr::ATTR_DESCRIPTION)
            .and_then(semantic_data::value::Value::as_str),
        Some("untouched")
    );
    let updated = update_task(
        &store,
        UpdateTaskPayload {
            id: task.id,
            clear_due_date: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(updated.due_date.is_none());
    assert_eq!(updated.created_at, task.created_at);
}
#[tokio::test]
async fn content_conversion_and_service_reject_invalid_inputs() {
    let store = setup().await;
    let task = create_task(&store, create("Valid", None)).await.unwrap();
    assert!(
        update_task(
            &store,
            UpdateTaskPayload {
                id: task.id.clone(),
                progress: Some(101),
                ..Default::default()
            }
        )
        .await
        .is_err()
    );
    let mut object = task.to_object();
    let mut content = Object::new();
    content.insert("kind", "note".to_owned().into_value());
    content.insert("data", semantic_data::value::Value::Object(Object::new()));
    object.insert(
        crate::content::ATTR_MAIN_CONTENT,
        semantic_data::value::Value::Object(content),
    );
    assert!(
        MainContent::from_value(
            object
                .get(crate::content::ATTR_MAIN_CONTENT)
                .unwrap()
                .clone()
        )
        .is_err()
    );
    let mut content = MainContent::note("body");
    let MainContent::Note { format, .. } = &mut content;
    *format = "invalid".into();
    assert!(
        update_task(
            &store,
            UpdateTaskPayload {
                id: task.id,
                main_content: Some(content),
                ..Default::default()
            }
        )
        .await
        .is_err()
    );
}
#[tokio::test]
async fn same_subject_id_in_different_collections_is_isolated() {
    let store = setup().await;
    let subject = create_task(&store, create("Subject", None)).await.unwrap();
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
        .insert("other", &subject.id, subject.to_object())
        .await
        .unwrap();
    let left = EntityTarget {
        collection: DEFAULT_COLLECTION.into(),
        id: subject.id.clone(),
    };
    let right = EntityTarget {
        collection: "other".into(),
        id: subject.id,
    };
    let comment = create_comment(
        &store,
        CreateCommentPayload {
            scope_id: None,
            target: left.clone(),
            main_content: MainContent::note("Left"),
            parent: None,
        },
        "alice".into(),
    )
    .await
    .unwrap();
    assert_eq!(
        list_comments(
            &store,
            ListCommentsPayload {
                scope_id: None,
                target: right.clone(),
                offset: 0,
                limit: 50
            }
        )
        .await
        .unwrap()
        .total,
        0
    );
    assert!(
        create_comment(
            &store,
            CreateCommentPayload {
                scope_id: None,
                target: right.clone(),
                main_content: MainContent::note("Wrong parent"),
                parent: Some(comment.id)
            },
            "alice".into()
        )
        .await
        .is_err()
    );
    create_comment(
        &store,
        CreateCommentPayload {
            scope_id: None,
            target: right.clone(),
            main_content: MainContent::note("Right"),
            parent: None,
        },
        "alice".into(),
    )
    .await
    .unwrap();
    for target in [left, right] {
        assert_eq!(
            list_comments(
                &store,
                ListCommentsPayload {
                    scope_id: None,
                    target,
                    offset: 0,
                    limit: 50
                }
            )
            .await
            .unwrap()
            .total,
            1
        );
    }
}
#[tokio::test]
async fn task_filters_sort_and_pagination_are_stable() {
    let store = setup().await;
    let alpha = create_task(&store, create("Alpha", None)).await.unwrap();
    create_task(&store, create("Beta", None)).await.unwrap();
    let page = list_tasks(
        &store,
        TaskListPayload {
            sort: Some("title".into()),
            limit: 1,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(page.total, 2);
    assert_eq!(page.tasks[0].id, alpha.id);
    let page = list_tasks(
        &store,
        TaskListPayload {
            sort: Some("title".into()),
            offset: 1,
            limit: 1,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(page.tasks[0].title, "Beta");
    assert_eq!(
        list_tasks(
            &store,
            TaskListPayload {
                search: Some("ALP".into()),
                priority: Some(TaskPriority::Medium),
                limit: 50,
                ..Default::default()
            }
        )
        .await
        .unwrap()
        .total,
        1
    );
    assert!(
        list_tasks(
            &store,
            TaskListPayload {
                sort: Some("title; DROP".into()),
                limit: 50,
                ..Default::default()
            }
        )
        .await
        .is_err()
    );
}
