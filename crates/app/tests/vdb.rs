use std::{
    collections::BTreeMap,
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

use semantic_app::{AppRequestContext, DbScopeId, Principal, SemanticApp, SemanticDb};
use semantic_data::{
    FromValue, Object, Value,
    query::{self as q, QueryInput},
    schema::{AttributeRef, AttributeType, ClassAttribute, DbOpenMode, Type},
};
use semantic_db_core::{Batch, BatchOperation, Db, QueryResult};
use semantic_plugin::Plugin;
use semantic_vdb::testing::{
    FixtureVdb, NegotiationMode, broken_fixture_schema, fixture_plugin_with_vdb, fixture_schema,
};

fn schema() -> semantic_vdb::DatabaseSchema {
    let mut schema = fixture_schema();
    for attribute in &mut schema.attributes {
        attribute.name = format!("fixture_{}", attribute.name);
    }
    for class in &mut schema.classes {
        class.attributes = std::mem::take(&mut class.attributes)
            .into_iter()
            .map(|(alias, attribute)| (format!("fixture_{alias}"), attribute))
            .collect();
    }
    schema
}

const CORPUS: [&str; 22] = [
    "SELECT id FROM {coll} ORDER BY id",
    "SELECT id, fixture_name FROM {coll} ORDER BY id",
    "SELECT id FROM {coll} WHERE fixture_name = 'Alpha' ORDER BY id",
    "SELECT id FROM {coll} WHERE fixture_name LIKE 'A%' ORDER BY id",
    "SELECT id FROM {coll} WHERE fixture_score BETWEEN 10 AND 30 ORDER BY id",
    "SELECT id FROM {coll} WHERE fixture_score >= 20 ORDER BY id",
    "SELECT id FROM {coll} WHERE fixture_score >= 20 AND fixture_active = true ORDER BY id",
    "SELECT id FROM {coll} WHERE fixture_score < 20 OR fixture_active = false ORDER BY id",
    "SELECT id FROM {coll} WHERE NOT fixture_active ORDER BY id",
    "SELECT id FROM {coll} WHERE fixture_category IS NULL ORDER BY id",
    "SELECT id FROM {coll} WHERE type = 'fixture:Tag' ORDER BY id",
    "SELECT id FROM {coll} ORDER BY fixture_category, fixture_score DESC",
    "SELECT id FROM {coll} ORDER BY fixture_score DESC LIMIT 2",
    "SELECT id FROM {coll} ORDER BY fixture_score LIMIT 2 OFFSET 1",
    "SELECT DISTINCT fixture_category FROM {coll} ORDER BY fixture_category",
    "SELECT fixture_category, COUNT(*) AS count FROM {coll} GROUP BY fixture_category ORDER BY fixture_category",
    "SELECT a.id FROM {coll} a JOIN {coll}._ b ON a.id = b.id ORDER BY a.id",
    "SELECT id FROM {coll} WHERE id IN (SELECT v.id FROM {coll} v WHERE v.fixture_score > 20) ORDER BY id",
    "SELECT a.id, b.fixture_title AS fixture_title FROM peers a JOIN {coll}._ b ON a.id = b.id ORDER BY a.id",
    "SELECT a.id FROM {coll} a JOIN {other}._ b ON a.id = b.id WHERE b.\"fixture:name\" = 'Alpha' ORDER BY a.id",
    "SELECT a.id FROM {coll} a WHERE EXISTS (SELECT l.id FROM peers l WHERE l.id = 'a') AND a.id IN (SELECT l.id FROM peers l) ORDER BY a.id",
    "SELECT a.id FROM {coll} a WHERE EXISTS (SELECT v.id FROM {other} v WHERE v.id = 'a') AND a.id IN (SELECT v.id FROM {other} v) ORDER BY a.id",
];
#[test]
fn corpus_sql_and_plain_aliases_preflight() {
    use semantic_db_core::{
        catalog::{Catalog, CollectionKind, IntegrityMode},
        sql::{SqlDialectKind, parse_sql_query_unbound},
    };
    let catalog = Catalog::new();
    let shared = semantic_vdb::DatabaseSchema {
        attributes: semantic_data::bundles::shared::module()
            .attributes
            .into_values()
            .collect(),
        ..Default::default()
    };
    let (mut catalog, _) =
        semantic_db_core::apply_ddl_batch(&catalog, &shared.to_ddl_batch()).unwrap();
    for name in ["mirror", "mirror2", "peers"] {
        catalog
            .upsert_collection(name, CollectionKind::Polymorphic, IntegrityMode::Permissive)
            .unwrap();
    }
    let (catalog, _) =
        semantic_db_core::apply_ddl_batch(&catalog, &schema().to_ddl_batch()).unwrap();
    for sql in CORPUS
        .into_iter()
        .map(|sql| {
            sql.replace("{coll}", "mirror")
                .replace("{other}", "mirror2")
        })
        .chain(["SELECT id FROM mirror WHERE fixture_name = :name".into()])
    {
        let query = parse_sql_query_unbound(&sql, SqlDialectKind::Generic)
            .unwrap_or_else(|error| panic!("{sql}: {error}"));
        let bindings = if sql.contains(" = :name") {
            BTreeMap::from([("name".into(), Value::String("Alpha".into()))])
        } else {
            BTreeMap::new()
        };
        let query = query.into_bound(&bindings).unwrap();
        let query: semantic_db_core::Query = query.into();
        let semantic_db_core::Query::Select(query) = query else {
            panic!("SELECT")
        };
        let collection = catalog
            .collection_by_name(query.collection_or_default())
            .unwrap();
        semantic_db_core::canonicalize_select_query(&query, &catalog, collection)
            .unwrap_or_else(|error| panic!("{sql}: {error}"));
    }
    for sql in [
        "INSERT INTO fx (id) VALUES ('z')",
        "UPDATE fx SET fixture_name = 'Changed'",
        "DELETE FROM fx",
    ] {
        parse_sql_query_unbound(sql, SqlDialectKind::Generic)
            .unwrap_or_else(|error| panic!("{sql}: {error}"));
    }
}

#[tokio::test]
async fn stored_corpus_smoke_and_qualified_join_predicate() {
    let temp = tempfile::tempdir().unwrap();
    let db: Arc<dyn SemanticDb> = Arc::new(Db::new(
        semantic_db_redb::open_backend(temp.path().join("join.redb"), DbOpenMode::AutoCreate)
            .unwrap(),
    ));
    let shared = semantic_vdb::DatabaseSchema {
        attributes: semantic_data::bundles::shared::module()
            .attributes
            .into_values()
            .collect(),
        ..Default::default()
    };
    db.query_data(
        q::DdlQuery {
            batch: shared.to_ddl_batch(),
        }
        .into(),
    )
    .await
    .unwrap();
    db.query_data(
        q::DdlQuery {
            batch: schema().to_ddl_batch(),
        }
        .into(),
    )
    .await
    .unwrap();
    let data = entities();
    populate(&db, "mirror", &data).await;
    populate(&db, "mirror2", &data).await;
    populate(&db, "peers", &data).await;
    for sql in CORPUS {
        rows(
            &db,
            &sql.replace("{coll}", "mirror")
                .replace("{other}", "mirror2"),
        )
        .await;
    }
    let mut expected = Object::new();
    expected.insert("id", "a".to_owned());
    assert_eq!(
        rows(
            &db,
            "SELECT a.id FROM mirror a JOIN mirror2._ b ON a.id = b.id WHERE b.\"fixture:name\" = 'Alpha' ORDER BY a.id",
        )
        .await,
        vec![expected],
    );
}

fn context(app: SemanticApp) -> AppRequestContext {
    AppRequestContext {
        app,
        principal: Principal::system(),
        session: None,
        request_scope: None,
    }
}

fn entities() -> Vec<Object> {
    [
        ("a", "Alpha", Some("red"), 10, true),
        ("b", "Beta", Some("blue"), 20, false),
        ("c", "Gamma", Some("red"), 30, true),
        ("d", "Delta", None, 40, false),
    ]
    .into_iter()
    .map(|(id, name, category, score, active)| {
        let mut row = Object::new();
        row.insert("id", id.to_owned());
        row.insert(
            "type",
            if id == "d" {
                "fixture:Tag"
            } else {
                "fixture:Item"
            }
            .to_owned(),
        );
        row.insert("fixture:name", name.to_owned());
        row.insert("semantic:title", name.to_owned());
        row.insert("fixture:score", Value::U64(score));
        row.insert("fixture:active", active);
        if let Some(category) = category {
            row.insert("fixture:category", category.to_owned());
        }
        row
    })
    .collect()
}

async fn collection(db: &Arc<dyn SemanticDb>, name: &str) {
    db.query_data(
        q::DdlQuery {
            batch: q::DdlBatch {
                operations: vec![q::DdlOperation::UpsertCollection {
                    name: name.into(),
                    kind: q::DdlCollectionKind::Polymorphic,
                    integrity_mode: q::IntegrityMode::Permissive,
                }],
            },
        }
        .into(),
    )
    .await
    .unwrap();
}

async fn populate(db: &Arc<dyn SemanticDb>, name: &str, rows: &[Object]) {
    collection(db, name).await;
    for row in rows {
        db.insert(
            name.into(),
            row.get("id").unwrap().as_str().unwrap().into(),
            row.clone(),
        )
        .await
        .unwrap();
    }
}

async fn rows(db: &Arc<dyn SemanticDb>, sql: &str) -> Vec<Object> {
    match db
        .query_data(QueryInput::sql(sql))
        .await
        .unwrap_or_else(|error| panic!("{sql}: {error}"))
    {
        QueryResult::Select(rows) => rows,
        result => panic!("expected SELECT: {result:?}"),
    }
}

#[tokio::test]
async fn stored_local_oracle_matches_four_plugin_modes_and_runtime_lifecycle() {
    let temp = tempfile::tempdir().unwrap();
    let local: Arc<dyn SemanticDb> = Arc::new(Db::new(
        semantic_db_redb::open_backend(temp.path().join("oracle.redb"), DbOpenMode::AutoCreate)
            .unwrap(),
    ));
    let runtime: Arc<dyn SemanticDb> = Arc::new(Db::new(
        semantic_db_redb::open_backend(temp.path().join("runtime.redb"), DbOpenMode::AutoCreate)
            .unwrap(),
    ));
    let oracle_app = SemanticApp::builder()
        .with_default_scope(DbScopeId::new("main"), local.clone())
        .build()
        .unwrap();
    let oracle_ctx = context(oracle_app.clone());
    let oracle = oracle_ctx.resolve_db(None).await.unwrap();
    oracle
        .query_data(
            q::DdlQuery {
                batch: schema().to_ddl_batch(),
            }
            .into(),
        )
        .await
        .unwrap();
    let data = entities();
    populate(&oracle, "mirror", &data).await;
    populate(&oracle, "mirror2", &data).await;
    populate(&oracle, "peers", &data).await;

    let mut expected = Vec::new();
    for sql in &CORPUS {
        expected.push(
            rows(
                &oracle,
                &sql.replace("{coll}", "mirror")
                    .replace("{other}", "mirror2"),
            )
            .await,
        );
    }
    eprintln!("stored oracle: {} queries ready", CORPUS.len());

    let modes = [
        NegotiationMode::AllExact,
        NegotiationMode::AllInexact,
        NegotiationMode::AllUnsupported,
        NegotiationMode::Alternating,
    ];
    let fixtures: Vec<_> = modes
        .into_iter()
        .map(|mode| FixtureVdb::new(data.clone(), mode).with_schema(schema(), "1"))
        .collect();
    let reject = FixtureVdb::new(data.clone(), NegotiationMode::AllExact)
        .with_schema(schema(), "1")
        .with_reject_without("type");
    let broken = FixtureVdb::new(data.clone(), NegotiationMode::AllExact)
        .with_schema(broken_fixture_schema(), "1");
    let slow = FixtureVdb::new(data.clone(), NegotiationMode::AllUnsupported)
        .with_schema(schema(), "1")
        .with_scan_gate(Arc::new(tokio::sync::Notify::new()));
    let requires_id = FixtureVdb::new(data.clone(), NegotiationMode::AllExact)
        .with_schema(schema(), "1")
        .with_reject_without("id");
    // Resolve the interface catalog once; configuration selects the honest
    // fixture implementation for each activation/generation.
    let manifest = fixture_plugin_with_vdb("fx", fixtures[0].clone())
        .manifest()
        .clone();
    let databases = Arc::new(
        fixtures
            .iter()
            .cloned()
            .chain([reject.clone(), broken, slow.clone(), requires_id.clone()])
            .collect::<Vec<_>>(),
    );
    let plugin = semantic_vdb::VirtualDatabasePlugin::new(
        manifest,
        move |context: semantic_plugin::PluginInstanceContext| {
            let index = match context.configuration {
                Value::U64(index) => index as usize,
                _ => 0,
            };
            let fixture = databases[index].clone();
            fixture.activations.fetch_add(1, Ordering::SeqCst);
            async move { Ok(fixture) }
        },
    );
    let app = SemanticApp::builder()
        .with_default_scope(DbScopeId::new("main"), runtime.clone())
        .register_builtin_commands()
        .unwrap()
        .register_plugin(plugin)
        .unwrap()
        .build()
        .unwrap();
    let ctx = context(app.clone());
    let federated = ctx.resolve_db(None).await.unwrap();
    // Local operations do not construct any plugin implementation.
    collection(&runtime, "peers").await;
    assert_eq!(rows(&federated, "SELECT id FROM peers").await.len(), 0);
    assert!(
        fixtures
            .iter()
            .all(|fixture| fixture.activations.load(Ordering::SeqCst) == 0)
    );
    // Local peers carry built-in attributes, independent of virtual fixture schema.
    for row in &data {
        let mut peer = Object::new();
        peer.insert("id", row.get("id").unwrap().clone());
        peer.insert("semantic:title", row.get("semantic:title").unwrap().clone());
        // Permissive untyped rows avoid requiring fixture classes locally.
        federated
            .insert(
                "peers".into(),
                row.get("id").unwrap().as_str().unwrap().into(),
                peer,
            )
            .await
            .unwrap();
    }
    let plugins = app
        .plugins(&Principal::system(), DbScopeId::new("main"))
        .await
        .unwrap();
    let baseline_migrations: Vec<_> = runtime
        .catalog()
        .await
        .unwrap()
        .applied_migrations()
        .map(|(key, _)| key.clone())
        .collect();
    let activations = plugins.list().await.unwrap();
    for i in 0..fixtures.len() {
        let mut activation = activations
            .iter()
            .find(|activation| activation.id == "fx")
            .unwrap()
            .clone();
        activation.configuration = Value::U64(i as u64);
        activation.id = "fx".into();
        plugins.configure(activation.clone()).await.unwrap();
        activation.id = "fx2".into();
        plugins.configure(activation).await.unwrap();
        for (sql, expected) in CORPUS.iter().zip(&expected) {
            let actual = rows(
                &federated,
                &sql.replace("{coll}", "fx").replace("{other}", "fx2"),
            )
            .await;
            assert_eq!(&actual, expected, "mode {:?}: {sql}", modes[i]);
        }
        // The embedded planner does not recanonicalize a stripped RHS alias
        // after join-predicate pushdown. The differential query uses its
        // qualified attribute id, while federation's plain alias has a direct
        // expected result so it remains covered in every negotiation mode.
        let mut expected = Object::new();
        expected.insert("id", "a".to_owned());
        assert_eq!(rows(&federated, "SELECT a.id FROM fx a JOIN fx2._ b ON a.id = b.id WHERE b.fixture_name = 'Alpha' ORDER BY a.id").await, vec![expected]);
        eprintln!(
            "{:?}: {} comparisons and plain join alias passed",
            modes[i],
            CORPUS.len()
        );
    }
    let ast = runtime
        .parse_sql("SELECT id FROM fx WHERE fixture_name = :name".into())
        .await
        .unwrap();
    let result = federated
        .query_data(QueryInput::ast_with_params(
            ast,
            BTreeMap::from([("name".into(), Value::String("Alpha".into()))]),
        ))
        .await
        .unwrap();
    assert_eq!(
        result,
        QueryResult::Select(
            rows(
                &oracle,
                "SELECT id FROM mirror WHERE fixture_name = 'Alpha'"
            )
            .await
        )
    );
    assert!(
        federated
            .get("fx".into(), "a".into())
            .await
            .unwrap()
            .is_some()
    );
    for (index, name) in [(4, "reject"), (5, "broken"), (6, "slow"), (7, "bound")] {
        let mut activation = activations
            .iter()
            .find(|activation| activation.id == "fx")
            .unwrap()
            .clone();
        activation.id = name.into();
        activation.configuration = Value::U64(index);
        plugins.configure(activation).await.unwrap();
    }
    assert_sdk_bind_join(ctx.clone(), federated.clone(), requires_id).await;
    assert!(
        federated
            .query_data(QueryInput::sql("SELECT id FROM reject"))
            .await
            .unwrap_err()
            .to_string()
            .contains("requires a filter")
    );
    for sql in [
        "INSERT INTO fx (id) VALUES ('z')",
        "UPDATE fx SET fixture_name = 'Changed'",
        "DELETE FROM fx",
    ] {
        assert!(
            federated
                .query_data(QueryInput::sql(sql))
                .await
                .unwrap_err()
                .to_string()
                .contains("read-only virtual database")
        );
    }
    assert!(
        federated
            .execute_batch(Batch {
                operations: vec![BatchOperation::DeleteById {
                    collection: "fx".into(),
                    id: "a".into()
                }]
            })
            .await
            .unwrap_err()
            .to_string()
            .contains("read-only virtual database")
    );
    assert_runtime_schema_lifecycle(
        ctx.clone(),
        runtime.clone(),
        federated.clone(),
        fixtures[3].clone(),
        slow.clone(),
        baseline_migrations,
    )
    .await;
    app.shutdown().await.unwrap();
    oracle_app.shutdown().await.unwrap();
}

async fn assert_sdk_bind_join(
    ctx: AppRequestContext,
    federated: Arc<dyn SemanticDb>,
    fixture: FixtureVdb,
) {
    let scans = fixture.scans.load(Ordering::SeqCst);
    assert!(
        federated
            .query_data(QueryInput::sql("SELECT id FROM bound"))
            .await
            .unwrap_err()
            .to_string()
            .contains("requires a filter on 'id'")
    );
    assert_eq!(fixture.scans.load(Ordering::SeqCst), scans);

    const SQL: &str = "SELECT l.id AS left_id, r.id AS right_id FROM peers l JOIN bound r ON l.id = r.id ORDER BY l.id";
    let mut payload = Object::new();
    payload.insert("query", SQL.to_owned());
    let explanation = ctx
        .app
        .call(ctx.clone(), "semantic.vdb.explain", Value::Object(payload))
        .await
        .unwrap();
    assert!(
        explanation
            .get_field("physical")
            .and_then(Value::as_str)
            .unwrap()
            .contains("IndexNestedLoopJoin")
    );
    let Value::List(leaves) = explanation.get_field("leaves").unwrap() else {
        panic!("explain leaves")
    };
    let leaf = leaves
        .iter()
        .find(|leaf| leaf.get_field("collection").and_then(Value::as_str) == Some("bound"))
        .unwrap();
    let Value::List(filters) = leaf.get_field("filters").unwrap() else {
        panic!("bound filters")
    };
    assert_eq!(filters.len(), 1);
    assert_eq!(
        q::Expr::from_value(filters[0].get_field("expression").unwrap().clone()).unwrap(),
        q::Expr::Binary {
            op: q::BinaryOp::In,
            left: Box::new(q::Expr::Operand(q::Operand::Field(
                semantic_data::value::FieldPath::from_fields(["id"]),
            ))),
            right: Box::new(q::Expr::parameter("__keys")),
        }
    );
    assert_eq!(
        semantic_vdb::FilterSupport::from_value(filters[0].get_field("support").unwrap().clone())
            .unwrap(),
        semantic_vdb::FilterSupport::Exact
    );
    // Explain negotiates the lookup but never scans. With batching enabled,
    // these four distinct peer keys must travel through the SDK in one list.
    assert_eq!(leaf.get_field("batch_size"), Some(&Value::U64(64)));
    assert_eq!(fixture.scans.load(Ordering::SeqCst), scans);
    let expected = ["a", "b", "c", "d"]
        .into_iter()
        .map(|id| {
            let mut row = Object::new();
            row.insert("left_id", id.to_owned());
            row.insert("right_id", id.to_owned());
            row
        })
        .collect::<Vec<_>>();
    assert_eq!(rows(&federated, SQL).await, expected);
    assert_eq!(fixture.scans.load(Ordering::SeqCst) - scans, 1);
    eprintln!("Native SDK selected bind join passed with one scan");
}

async fn assert_runtime_schema_lifecycle(
    ctx: AppRequestContext,
    runtime: Arc<dyn SemanticDb>,
    federated: Arc<dyn SemanticDb>,
    fixture: FixtureVdb,
    slow: FixtureVdb,
    baseline_migrations: Vec<String>,
) {
    let app = ctx.app.clone();
    let plugins = app
        .plugins(&Principal::system(), DbScopeId::new("main"))
        .await
        .unwrap();
    let before = runtime.catalog().await.unwrap();
    assert!(before.collection_by_name("fx").is_none());
    assert!(before.class_id("fixture:Item").is_none());
    assert_eq!(
        baseline_migrations,
        before
            .applied_migrations()
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>()
    );
    let mut payload = Object::new();
    payload.insert("name", "fx".to_owned());
    assert!(
        app.call(ctx.clone(), "semantic.vdb.schema", Value::Object(payload))
            .await
            .unwrap()
            .get_field("classes")
            .is_some()
    );
    let list = app
        .call(
            ctx.clone(),
            "semantic.vdb.list",
            Value::Object(Object::new()),
        )
        .await
        .unwrap();
    let Value::List(list) = list else {
        panic!("list")
    };
    let bad = list
        .iter()
        .find(|entry| entry.get_field("name").and_then(Value::as_str) == Some("broken"))
        .unwrap();
    assert_eq!(bad.get_field("available"), Some(&Value::Bool(false)));
    assert!(bad.get_field("reason").and_then(Value::as_str).is_some());
    eprintln!("Runtime schema isolation and unavailable-source listing passed");
    // A revision change refreshes once, including an unavailable refresh diagnostic.
    let describes = fixture.describes.load(Ordering::SeqCst);
    {
        let mut schema = fixture.schema.lock().unwrap();
        schema.1 = "2".into();
        schema.0.attributes.push(AttributeType {
            id: "fixture:added".into(),
            name: "fixture_added".into(),
            ty: Type::new(semantic_data::schema::TypeKind::String(
                semantic_data::schema::StringType {
                    format: None,
                    normalization: None,
                },
            )),
            constraints: vec![],
            meta: Default::default(),
        });
        for class in &mut schema.0.classes {
            class.attributes.insert(
                "fixture_added".into(),
                ClassAttribute {
                    attribute: AttributeRef {
                        id: "fixture:added".into(),
                    },
                    required: false,
                    ui_order: None,
                    computed: None,
                    default: None,
                    constraints: vec![],
                    meta: Default::default(),
                },
            );
        }
    }
    assert_eq!(
        rows(&federated, "SELECT id FROM fx WHERE fixture_added IS NULL")
            .await
            .len(),
        4
    );
    assert_eq!(fixture.describes.load(Ordering::SeqCst), describes + 1);
    *fixture.schema.lock().unwrap() = (broken_fixture_schema(), "3".into());
    assert!(
        federated
            .query_data(QueryInput::sql("SELECT id FROM fx"))
            .await
            .unwrap_err()
            .to_string()
            .contains("unavailable")
    );
    *fixture.schema.lock().unwrap() = (schema(), "4".into());
    let mut activation = plugins
        .list()
        .await
        .unwrap()
        .into_iter()
        .find(|activation| activation.id == "fx")
        .unwrap();
    activation.enabled = false;
    plugins.configure(activation.clone()).await.unwrap();
    assert!(
        federated
            .query_data(QueryInput::sql("SELECT id FROM fx"))
            .await
            .unwrap_err()
            .to_string()
            .contains("unavailable")
    );
    let Value::List(list) = app
        .call(
            ctx.clone(),
            "semantic.vdb.list",
            Value::Object(Object::new()),
        )
        .await
        .unwrap()
    else {
        panic!("list")
    };
    assert!(
        list.iter()
            .all(|entry| entry.get_field("name").and_then(Value::as_str) != Some("fx"))
    );
    activation.enabled = true;
    plugins.configure(activation).await.unwrap();
    assert_eq!(rows(&federated, "SELECT id FROM fx").await.len(), 4);
    // Reconfiguration cancels a live scan from the old generation.
    let cancelled = slow.cancelled.notified();
    tokio::pin!(cancelled);
    let db = federated.clone();
    let task =
        tokio::spawn(async move { db.query_data(QueryInput::sql("SELECT id FROM slow")).await });
    tokio::time::timeout(Duration::from_secs(10), async {
        while slow.scans.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let activation = plugins
        .list()
        .await
        .unwrap()
        .into_iter()
        .find(|activation| activation.id == "slow")
        .unwrap();
    plugins.configure(activation).await.unwrap();
    tokio::time::timeout(Duration::from_secs(10), cancelled)
        .await
        .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_secs(10), task)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
    // Local collection precedence remains authoritative, with conflict diagnostics.
    collection(&runtime, "fx").await;
    assert!(rows(&federated, "SELECT id FROM fx").await.is_empty());
    let Value::List(list) = app
        .call(
            ctx.clone(),
            "semantic.vdb.list",
            Value::Object(Object::new()),
        )
        .await
        .unwrap()
    else {
        panic!("list")
    };
    let entry = list
        .iter()
        .find(|entry| entry.get_field("name").and_then(Value::as_str) == Some("fx"))
        .unwrap();
    assert_eq!(entry.get_field("available"), Some(&Value::Bool(false)));
    assert!(
        entry
            .get_field("reason")
            .and_then(Value::as_str)
            .unwrap()
            .contains("local collection")
    );
}
