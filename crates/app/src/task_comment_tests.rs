//! Real storage tests for the optional packages at the scoped RPC boundary.
use crate::*;
use semantic_base::{comments::*, content::MainContent, tasks::*};
use semantic_data::{
    builtin::DEFAULT_COLLECTION,
    schema::DbOpenMode,
    value::{FromValue, IntoValue, Object, Value},
};
use semantic_db_core::Db;
use semantic_rpc_core::RpcCommandSpec;
use std::{collections::BTreeMap, path::Path, sync::Arc};

struct FixedProvider(BTreeMap<String, Arc<Db>>);
#[async_trait::async_trait]
impl DbProvider for FixedProvider {
    fn scheme(&self) -> &str {
        "fixture"
    }
    async fn open(
        &self,
        request: DbOpenRequest,
        _: &Principal,
    ) -> Result<Arc<dyn SemanticDb>, AppError> {
        Ok(self.0.get(&request.uri).expect("known fixture URI").clone())
    }
}
fn database(path: &Path) -> Arc<Db> {
    Arc::new(Db::new(
        semantic_db_redb::open_backend(path, DbOpenMode::AutoCreate).unwrap(),
    ))
}
fn context(app: &SemanticApp, principal: Principal) -> AppRequestContext {
    AppRequestContext {
        app: app.clone(),
        principal,
        session: None,
        request_scope: None,
    }
}
async fn call<C: RpcCommandSpec>(app: &SemanticApp, payload: C::Payload) -> C::Output {
    C::Output::from_value(
        app.call(
            context(app, Principal::system()),
            C::NAME,
            payload.into_value(),
        )
        .await
        .unwrap(),
    )
    .unwrap()
}
async fn capabilities(app: &SemanticApp, scope: &str) -> Object {
    let value = Value::Object(Object::from_iter([(
        "scope_id".to_owned(),
        scope.to_owned().into_value(),
    )]));
    match app
        .call(
            context(app, Principal::system()),
            "semantic.app.capabilities",
            value,
        )
        .await
        .unwrap()
    {
        Value::Object(object) => object,
        other => panic!("Expected capabilities object: {other:?}"),
    }
}
async fn open(app: &SemanticApp, id: &str) {
    app.scopes()
        .open_scope(
            &Principal::system(),
            ScopeOpenOptions {
                scope_id: Some(DbScopeId::new(id)),
                request: DbOpenRequest {
                    uri: format!("fixture://{id}"),
                    mode: DbOpenMode::AutoCreate,
                },
                visibility: ScopeVisibility::System,
                set_current: false,
            },
        )
        .await
        .unwrap();
}
async fn install(db: &Db) {
    db.upsert_package(semantic_base::package()).await.unwrap();
    db.upsert_package(semantic_base::comments::package())
        .await
        .unwrap();
    db.upsert_package(semantic_base::tasks::package())
        .await
        .unwrap();
}
fn create_payload(scope: &str, title: &str) -> CreateTaskPayload {
    CreateTaskPayload {
        scope_id: Some(scope.into()),
        title: title.into(),
        main_content: MainContent::note("**Description**"),
        status: TaskStatus::Todo,
        priority: TaskPriority::High,
        progress: None,
        due_date: None,
        parent: None,
    }
}
fn target(id: &str) -> semantic_base::comments::EntityTarget {
    semantic_base::comments::EntityTarget {
        collection: DEFAULT_COLLECTION.into(),
        id: id.into(),
    }
}
fn list_payload(scope: &str, id: &str) -> ListCommentsPayload {
    ListCommentsPayload {
        scope_id: Some(scope.into()),
        target: target(id),
        offset: 0,
        limit: 100,
    }
}

#[tokio::test]
async fn opened_scope_requires_explicit_optional_package_installation() {
    let temp = tempfile::tempdir().unwrap();
    let db = database(&temp.path().join("opened.redb"));
    let app = SemanticApp::builder()
        .with_provider(FixedProvider(BTreeMap::from([(
            "fixture://opened".into(),
            db.clone(),
        )])))
        .register_builtin_commands()
        .unwrap()
        .build()
        .unwrap();
    open(&app, "opened").await;
    let before = capabilities(&app, "opened").await;
    assert_eq!(before.get("tasks"), Some(&Value::Bool(false)));
    assert_eq!(before.get("comments"), Some(&Value::Bool(false)));
    assert!(
        db.catalog()
            .await
            .unwrap()
            .class_id("semantic:tasks:task")
            .is_none()
    );
    install(&db).await;
    let after = capabilities(&app, "opened").await;
    assert_eq!(after.get("tasks"), Some(&Value::Bool(true)));
    assert_eq!(after.get("comments"), Some(&Value::Bool(true)));
    let task = call::<CreateTask>(&app, create_payload("opened", "Installed explicitly")).await;
    assert_eq!(task.title, "Installed explicitly");
}

#[tokio::test]
async fn scoped_task_and_comment_commands_do_not_read_or_write_another_scope() {
    let temp = tempfile::tempdir().unwrap();
    let a = database(&temp.path().join("a.redb"));
    let b = database(&temp.path().join("b.redb"));
    install(&a).await;
    install(&b).await;
    let app = SemanticApp::builder()
        .with_provider(FixedProvider(BTreeMap::from([
            ("fixture://a".into(), a.clone()),
            ("fixture://b".into(), b.clone()),
        ])))
        .register_builtin_commands()
        .unwrap()
        .build()
        .unwrap();
    open(&app, "a").await;
    open(&app, "b").await;
    let first = call::<CreateTask>(&app, create_payload("a", "Scope A")).await;
    let empty = call::<ListTasks>(
        &app,
        TaskListPayload {
            scope_id: Some("b".into()),
            limit: 100,
            ..Default::default()
        },
    )
    .await;
    assert_eq!(empty.total, 0);
    let wrong_update = UpdateTaskPayload {
        scope_id: Some("b".into()),
        id: first.id.clone(),
        title: Some("Wrong scope".into()),
        ..Default::default()
    };
    assert!(
        app.call(
            context(&app, Principal::system()),
            UpdateTask::NAME,
            wrong_update.into_value()
        )
        .await
        .is_err()
    );
    let mut wrong_parent = create_payload("b", "Cross scope child");
    wrong_parent.parent = Some(first.id.clone());
    assert!(
        app.call(
            context(&app, Principal::system()),
            CreateTask::NAME,
            wrong_parent.into_value()
        )
        .await
        .is_err()
    );
    // Identical subject IDs in separate scope databases remain independent.
    b.insert(DEFAULT_COLLECTION, first.id.clone(), first.to_object())
        .await
        .unwrap();
    let comment = call::<CreateComment>(
        &app,
        CreateCommentPayload {
            scope_id: Some("a".into()),
            target: target(&first.id),
            main_content: MainContent::note("Only scope A"),
            parent: None,
        },
    )
    .await;
    assert_eq!(
        call::<ListComments>(&app, list_payload("a", &first.id))
            .await
            .total,
        1
    );
    assert_eq!(
        call::<ListComments>(&app, list_payload("b", &first.id))
            .await
            .total,
        0
    );
    let edit = EditCommentPayload {
        scope_id: Some("b".into()),
        id: comment.id.clone(),
        main_content: MainContent::note("Wrong scope edit"),
    };
    assert!(
        app.call(
            context(&app, Principal::system()),
            EditComment::NAME,
            edit.into_value()
        )
        .await
        .is_err()
    );
    let delete = DeleteCommentPayload {
        scope_id: Some("b".into()),
        id: comment.id.clone(),
    };
    assert!(
        app.call(
            context(&app, Principal::system()),
            DeleteComment::NAME,
            delete.into_value()
        )
        .await
        .is_err()
    );
    let reply = CreateCommentPayload {
        scope_id: Some("b".into()),
        target: target(&first.id),
        main_content: MainContent::note("Cross scope reply"),
        parent: Some(comment.id.clone()),
    };
    assert!(
        app.call(
            context(&app, Principal::system()),
            CreateComment::NAME,
            reply.into_value()
        )
        .await
        .is_err()
    );
    let preserved = call::<ListComments>(&app, list_payload("a", &first.id)).await;
    assert_eq!(preserved.comments[0].main_content.body(), "Only scope A");
    assert!(!preserved.comments[0].deleted);
    assert_eq!(
        call::<GetTask>(
            &app,
            TaskIdPayload {
                scope_id: Some("a".into()),
                id: first.id
            }
        )
        .await
        .task
        .title,
        "Scope A"
    );
}

#[tokio::test]
async fn disabling_handlers_hides_persisted_optional_schema_without_removing_data() {
    let temp = tempfile::tempdir().unwrap();
    let db = database(&temp.path().join("persisted.redb"));
    install(&db).await;
    let enabled = SemanticApp::builder()
        .with_provider(FixedProvider(BTreeMap::from([(
            "fixture://persisted".into(),
            db.clone(),
        )])))
        .register_builtin_commands()
        .unwrap()
        .build()
        .unwrap();
    open(&enabled, "persisted").await;
    let task = call::<CreateTask>(&enabled, create_payload("persisted", "Preserved task")).await;
    let comment = call::<CreateComment>(
        &enabled,
        CreateCommentPayload {
            scope_id: Some("persisted".into()),
            target: target(&task.id),
            main_content: MainContent::note("Preserved comment"),
            parent: None,
        },
    )
    .await;

    for (tasks, comments, expected_tasks, expected_comments) in [
        (false, true, false, true),
        (false, false, false, false),
        (true, false, true, true),
    ] {
        let config = AppConfig::new().with_tasks(tasks).with_comments(comments);
        let app = SemanticApp::builder()
            .with_config(config)
            .with_provider(FixedProvider(BTreeMap::from([(
                "fixture://persisted".into(),
                db.clone(),
            )])))
            .register_builtin_commands()
            .unwrap()
            .build()
            .unwrap();
        open(&app, "persisted").await;
        let available = capabilities(&app, "persisted").await;
        assert_eq!(available.get("tasks"), Some(&Value::Bool(expected_tasks)));
        assert_eq!(
            available.get("comments"),
            Some(&Value::Bool(expected_comments))
        );
        assert_eq!(
            app.registry().get(CreateTask::NAME).is_some(),
            expected_tasks
        );
        assert_eq!(
            app.registry().get(CreateComment::NAME).is_some(),
            expected_comments
        );
        let persisted_task = db
            .get(DEFAULT_COLLECTION, task.id.clone())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            Task::from_object(&persisted_task.object).unwrap().title,
            "Preserved task"
        );
        let persisted_comment = db
            .get(DEFAULT_COLLECTION, comment.id.clone())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            Comment::from_object(&persisted_comment.object)
                .unwrap()
                .main_content
                .body(),
            "Preserved comment"
        );
        if expected_comments {
            assert_eq!(
                call::<ListComments>(&app, list_payload("persisted", &task.id))
                    .await
                    .total,
                1
            );
        }

        assert!(
            db.catalog()
                .await
                .unwrap()
                .class_id("semantic:tasks:task")
                .is_some()
        );
        assert!(
            db.catalog()
                .await
                .unwrap()
                .class_id("semantic:comments:comment")
                .is_some()
        );
    }
}

#[tokio::test]
async fn comment_author_and_edit_authorization_use_request_principal() {
    let temp = tempfile::tempdir().unwrap();
    let db = database(&temp.path().join("ownership.redb"));
    install(&db).await;
    let app = SemanticApp::builder()
        .with_provider(FixedProvider(BTreeMap::from([(
            "fixture://ownership".into(),
            db,
        )])))
        .register_builtin_commands()
        .unwrap()
        .build()
        .unwrap();
    open(&app, "ownership").await;
    let task = call::<CreateTask>(&app, create_payload("ownership", "Shared subject")).await;
    let create = CreateCommentPayload {
        scope_id: Some("ownership".into()),
        target: target(&task.id),
        main_content: MainContent::note("Written by Alice"),
        parent: None,
    };
    let value = app
        .call(
            context(&app, Principal::user("alice")),
            CreateComment::NAME,
            create.into_value(),
        )
        .await
        .unwrap();
    let comment = Comment::from_value(value).unwrap();
    assert_eq!(comment.author_id, "alice");
    let edit = EditCommentPayload {
        scope_id: Some("ownership".into()),
        id: comment.id.clone(),
        main_content: MainContent::note("Unauthorized edit"),
    };
    assert!(
        app.call(
            context(&app, Principal::user("bob")),
            EditComment::NAME,
            edit.into_value()
        )
        .await
        .is_err()
    );
    let delete = DeleteCommentPayload {
        scope_id: Some("ownership".into()),
        id: comment.id.clone(),
    };
    assert!(
        app.call(
            context(&app, Principal::user("bob")),
            DeleteComment::NAME,
            delete.into_value()
        )
        .await
        .is_err()
    );
    let before = call::<ListComments>(&app, list_payload("ownership", &task.id)).await;
    assert_eq!(before.comments[0].main_content.body(), "Written by Alice");
    assert!(!before.comments[0].deleted);
    let edit = EditCommentPayload {
        scope_id: Some("ownership".into()),
        id: comment.id.clone(),
        main_content: MainContent::note("System moderation"),
    };
    let moderated = call::<EditComment>(&app, edit).await;
    assert_eq!(moderated.author_id, "alice");
    assert_eq!(moderated.main_content.body(), "System moderation");
}

#[tokio::test]
async fn default_scope_initializes_the_expected_packages_once() {
    let temp = tempfile::tempdir().unwrap();
    let db = database(&temp.path().join("default.redb"));
    let app = SemanticApp::builder()
        .with_default_scope(DbScopeId::new("default"), db.clone())
        .register_builtin_commands()
        .unwrap()
        .build()
        .unwrap();
    assert!(
        db.catalog()
            .await
            .unwrap()
            .class_id("semantic:tasks:task")
            .is_none()
    );
    for _ in 0..2 {
        let available = capabilities(&app, "default").await;
        assert_eq!(available.get("tasks"), Some(&Value::Bool(true)));
        assert_eq!(available.get("comments"), Some(&Value::Bool(true)));
        let catalog = db.catalog().await.unwrap();
        let actual: std::collections::BTreeSet<_> = catalog
            .packages()
            .map(|(_, registered)| registered.name.clone())
            .collect();
        let expected: std::collections::BTreeSet<_> = [
            semantic_base::PACKAGE_NAME,
            semantic_base::comments::PACKAGE_NAME,
            semantic_base::tasks::PACKAGE_NAME,
            semantic_data::filestore::PACKAGE_NAME,
            semantic_data::import::PACKAGE_NAME,
            semantic_data::bundles::query::PACKAGE_NAME,
            semantic_data::vdb::PACKAGE_NAME,
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        assert_eq!(actual, expected);
        assert!(catalog.class_id("semantic:base:note").is_some());
    }
}
