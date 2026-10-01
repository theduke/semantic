mod auth;
mod config;
mod error;
mod file;
mod interface;
mod router;
mod startup;
mod storage;
#[cfg(feature = "embed-ui")]
mod ui;

pub use auth::{HeaderPrincipalResolver, NoAuthPrincipalResolver, PrincipalResolver};
pub use config::ServerConfig;
pub use error::ServerError;
pub use router::SemanticServer;
#[cfg(feature = "redb")]
pub use semantic_app::storage::RedbDbProvider;
pub use semantic_app::storage::{LocalDbConfig, is_logfs_blob_uri};
#[cfg(feature = "logfs")]
pub use semantic_app::storage::{LogDbConfig, LogDbProvider, LogFsDbProvider};
pub use startup::prompt_blob_password;
pub use storage::resolve_db_uri;

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;
    use std::sync::Mutex;

    use async_trait::async_trait;
    use axum::body::{Body, to_bytes};
    use futures_util::StreamExt;
    use http::Request;
    use semantic_app::{DbScopeId, SemanticDb};
    use semantic_data::value::{Object, Value};
    use semantic_db_core::catalog::Catalog;
    use semantic_db_core::{
        Batch, BatchOutcome, DbError, EntityRecord, PackageRegistrationOutcome, QueryResult,
        TextQueryInput,
    };

    use semantic_rpc_core::{RpcRequest, RpcResponse, RpcResult};
    use tower::ServiceExt;

    use super::*;

    struct MockDb {
        name: String,
        records: Mutex<BTreeMap<(String, String), Object>>,
        catalog: Mutex<Catalog>,
    }

    #[async_trait]
    impl SemanticDb for MockDb {
        async fn catalog(&self) -> std::result::Result<Arc<Catalog>, DbError> {
            Ok(Arc::new(self.catalog.lock().unwrap().clone()))
        }

        async fn query(&self, _query: TextQueryInput) -> std::result::Result<QueryResult, DbError> {
            let mut row = Object::new();
            row.insert("db", Value::String(self.name.clone()));
            Ok(QueryResult::Select(vec![row]))
        }

        async fn get(
            &self,
            collection: String,
            id: String,
        ) -> std::result::Result<Option<EntityRecord>, DbError> {
            if let Some(object) = self
                .records
                .lock()
                .unwrap()
                .get(&(collection.clone(), id.clone()))
                .cloned()
            {
                return Ok(Some(EntityRecord {
                    collection,
                    id,
                    object,
                }));
            }
            Ok(Some(EntityRecord {
                collection,
                id,
                object: Object::new(),
            }))
        }

        async fn insert(
            &self,
            _collection: String,
            _id: String,
            _object: Object,
        ) -> std::result::Result<(), DbError> {
            self.records
                .lock()
                .unwrap()
                .insert((_collection, _id), _object);
            Ok(())
        }

        async fn delete(
            &self,
            _collection: String,
            _id: String,
        ) -> std::result::Result<(), DbError> {
            Ok(())
        }

        async fn execute_batch(&self, batch: Batch) -> std::result::Result<BatchOutcome, DbError> {
            let mut records = self.records.lock().unwrap();
            let mut dataset = semantic_db_core::Dataset::new();
            for ((collection, id), object) in records.iter() {
                dataset
                    .entry(collection.clone())
                    .or_default()
                    .insert(id.clone(), object.clone());
            }
            let outcome = semantic_db_core::execute_batch(&dataset, &batch).map_err(|error| {
                match error.entity_exists {
                    Some((collection, id)) => DbError::EntityExists { collection, id },
                    None => DbError::InvalidQuery(error.message),
                }
            })?;
            *records = outcome
                .dataset
                .iter()
                .flat_map(|(collection, rows)| {
                    rows.iter()
                        .map(move |(id, object)| ((collection.clone(), id.clone()), object.clone()))
                })
                .collect();
            Ok(outcome)
        }

        async fn upsert_package(
            &self,
            package: semantic_data::schema::Package,
        ) -> std::result::Result<PackageRegistrationOutcome, DbError> {
            assert!(
                package.name == "semantic.base"
                    || package.name == semantic_data::filestore::PACKAGE_NAME
                    || package.name == semantic_data::import::PACKAGE_NAME
                    || package.name == semantic_data::plugin::PACKAGE_NAME
                    || package.name == semantic_data::jobs::PACKAGE_NAME
                    || package.name == "semantic.comments"
                    || package.name == "semantic.tasks"
            );
            self.catalog
                .lock()
                .unwrap()
                .upsert_package(semantic_db_core::normalize_package_definition(&package).unwrap());
            Ok(PackageRegistrationOutcome {
                executed_migrations: vec![],
            })
        }
    }

    fn mock_db(name: &str) -> Arc<dyn SemanticDb> {
        Arc::new(MockDb {
            name: name.to_string(),
            records: Mutex::new(BTreeMap::new()),
            catalog: Mutex::new(Catalog::new()),
        })
    }

    /// Echoes its input stream back, with the same end value.
    struct StreamEcho;

    impl semantic_rpc::stream_command::RpcStreamCommandSpec for StreamEcho {
        type Payload = ();
        type Input = semantic_data::value::StreamOf<String, u64>;
        type Output = semantic_data::value::StreamOf<String, u64>;
        type Error = semantic_app::AppError;

        const NAME: &'static str = "test.stream.echo";
    }

    impl semantic_rpc::stream_command::RpcStreamCommand<semantic_app::AppRequestContext>
        for StreamEcho
    {
        fn call<'a>(
            &'a self,
            _ctx: &'a semantic_app::AppRequestContext,
            _payload: (),
            input: semantic_rpc::stream_command::TypedStream<String, u64>,
            _cancel: semantic_rpc::interface::CancellationToken,
        ) -> futures_util::future::BoxFuture<
            'a,
            Result<semantic_rpc::stream_command::TypedStream<String, u64>, semantic_app::AppError>,
        > {
            Box::pin(async move {
                Ok(semantic_rpc::stream_command::TypedStream::from_events(
                    input,
                ))
            })
        }
    }

    /// Streams `0..payload` followed by the end value `"done"`.
    struct StreamCount;

    impl semantic_rpc::stream_command::RpcStreamCommandSpec for StreamCount {
        type Payload = u32;
        type Input = ();
        type Output = semantic_data::value::StreamOf<u32, String>;
        type Error = semantic_app::AppError;

        const NAME: &'static str = "test.stream.count";
    }

    impl semantic_rpc::stream_command::RpcStreamCommand<semantic_app::AppRequestContext>
        for StreamCount
    {
        fn call<'a>(
            &'a self,
            _ctx: &'a semantic_app::AppRequestContext,
            payload: u32,
            _input: (),
            _cancel: semantic_rpc::interface::CancellationToken,
        ) -> futures_util::future::BoxFuture<
            'a,
            Result<semantic_rpc::stream_command::TypedStream<u32, String>, semantic_app::AppError>,
        > {
            use semantic_rpc::stream_command::{TypedEvent, TypedStream};
            Box::pin(async move {
                let items = futures_util::stream::iter(0..payload)
                    .map(|item| Ok(TypedEvent::Item(item)))
                    .chain(futures_util::stream::once(async {
                        Ok(TypedEvent::End("done".to_owned()))
                    }));
                Ok(TypedStream::from_events(items))
            })
        }
    }

    /// Sums its input stream.
    struct StreamSum;

    impl semantic_rpc::stream_command::RpcStreamCommandSpec for StreamSum {
        type Payload = ();
        type Input = semantic_data::value::StreamOf<u32>;
        type Output = semantic_rpc::stream_command::Single<u64>;
        type Error = semantic_app::AppError;

        const NAME: &'static str = "test.stream.sum";
    }

    impl semantic_rpc::stream_command::RpcStreamCommand<semantic_app::AppRequestContext> for StreamSum {
        fn call<'a>(
            &'a self,
            _ctx: &'a semantic_app::AppRequestContext,
            _payload: (),
            mut input: semantic_rpc::stream_command::TypedStream<u32>,
            _cancel: semantic_rpc::interface::CancellationToken,
        ) -> futures_util::future::BoxFuture<'a, Result<u64, semantic_app::AppError>> {
            use semantic_rpc::stream_command::TypedEvent;
            Box::pin(async move {
                let mut total = 0;
                while let Some(event) = input.next().await {
                    match event
                        .map_err(|error| semantic_app::AppError::UnknownCommand(error.message))?
                    {
                        TypedEvent::Item(item) => total += u64::from(item),
                        TypedEvent::End(()) => break,
                    }
                }
                Ok(total)
            })
        }
    }

    fn test_app() -> semantic_app::SemanticApp {
        let default_db = mock_db("default");
        let header_db = mock_db("header");
        let query_db = mock_db("query");
        let data_dir = std::env::temp_dir().join("semantic-server-test-blob");
        let blob_uri = semantic_app::AppConfig::new()
            .with_data_dir(&data_dir)
            .default_blob_uri()
            .unwrap();
        let app = semantic_app::SemanticApp::builder()
            .with_default_scope(DbScopeId::new("default"), default_db)
            .with_default_file_store_uri(DbScopeId::new("default"), blob_uri)
            .register_builtin_commands()
            .unwrap()
            .register_stream_command(StreamEcho)
            .unwrap()
            .register_stream_command(StreamCount)
            .unwrap()
            .register_stream_command(StreamSum)
            .unwrap()
            .build()
            .unwrap();
        app.scopes()
            .add_default_scope(DbScopeId::new("header"), header_db)
            .unwrap();
        app.scopes()
            .add_default_scope(DbScopeId::new("query"), query_db)
            .unwrap();
        app
    }

    #[tokio::test]
    async fn interface_client_negotiates_application_schema_over_websocket() {
        assert_interface_client_path("/api/v1/rpc").await;
    }

    #[tokio::test]
    async fn interface_client_uses_configured_rpc_path() {
        for path in ["/custom/rpc", "/custom/commands", "/custom/commands/"] {
            assert_interface_client_path(path).await;
        }
    }

    async fn assert_interface_client_path(rpc_path: &str) {
        let app = test_app();
        let server = SemanticServer::new(app.clone()).with_config(ServerConfig {
            rpc_path: rpc_path.into(),
            ..Default::default()
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            axum::serve(listener, server.router()).await.unwrap();
        });
        let client: semantic_rpc::client::RpcClient =
            semantic_rpc::transport::http_client::HttpRpcClient::new(format!(
                "http://{address}{rpc_path}?scope=query"
            ))
            .into();
        let result = client
            .invoke_interface(semantic_rpc::interface::ValidatedInvocation {
                export: "application".into(),
                method: "unknown".into(),
                arguments: vec![],
            })
            .await;
        assert!(matches!(result, Err(error) if error.code == "invalid_argument"));
        drop(client);
        task.abort();
        app.shutdown().await.unwrap();
    }

    struct InterfaceFixture {
        client: semantic_rpc::client::RpcClient,
        task: tokio::task::JoinHandle<()>,
        app: semantic_app::SemanticApp,
    }

    impl InterfaceFixture {
        async fn start(server: impl FnOnce(SemanticServer) -> SemanticServer, query: &str) -> Self {
            let app = test_app();
            let server = server(SemanticServer::new(app.clone()));
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let task = tokio::spawn(async move {
                axum::serve(listener, server.router()).await.unwrap();
            });
            let client = semantic_rpc::transport::http_client::HttpRpcClient::new(format!(
                "http://{address}/api/v1/rpc{query}"
            ))
            .into();
            Self { client, task, app }
        }

        async fn invoke(
            &self,
            method: &str,
            arguments: Vec<semantic_rpc::interface::InvocationArgument>,
        ) -> Result<
            semantic_rpc::interface::InvocationOutput,
            semantic_rpc_core::interface::InvocationError,
        > {
            self.client
                .invoke_interface(semantic_rpc::interface::ValidatedInvocation {
                    export: semantic_rpc::stream_command::COMMAND_EXPORT.into(),
                    method: method.into(),
                    arguments,
                })
                .await
        }

        async fn stop(self) {
            drop(self.client);
            self.task.abort();
            self.app.shutdown().await.unwrap();
        }
    }

    fn payload(value: Value) -> semantic_rpc::interface::InvocationArgument {
        semantic_rpc::interface::InvocationArgument::Value(value)
    }

    fn stream_argument<T: semantic_data::value::IntoValue + Send + 'static>(
        items: impl IntoIterator<Item = T>,
    ) -> semantic_rpc::interface::InvocationArgument {
        use semantic_rpc::stream_command::{TypedEvent, TypedStream};
        let events: Vec<_> = items
            .into_iter()
            .map(|item| Ok(TypedEvent::Item(item)))
            .chain([Ok(TypedEvent::End(()))])
            .collect();
        semantic_rpc::interface::InvocationArgument::Stream(
            TypedStream::<T>::from_events(futures_util::stream::iter(events)).into_owned(),
        )
    }

    #[tokio::test]
    async fn command_export_streams_items_and_end_value_to_the_client() {
        use semantic_rpc::stream_command::{TypedEvent, TypedStream};
        let fixture = InterfaceFixture::start(|server| server, "").await;
        let semantic_rpc::interface::InvocationOutput::Stream(stream) = fixture
            .invoke("test.stream.count", vec![payload(Value::U32(150))])
            .await
            .unwrap()
        else {
            panic!("expected stream");
        };
        let events: Vec<_> = TypedStream::<u32, String>::from_owned(stream)
            .collect()
            .await;
        assert_eq!(events.len(), 151);
        for (index, event) in events[..150].iter().enumerate() {
            assert_eq!(event, &Ok(TypedEvent::Item(index as u32)));
        }
        assert_eq!(events[150], Ok(TypedEvent::End("done".to_owned())));
        fixture.stop().await;
    }

    #[tokio::test]
    async fn command_export_accepts_client_streams_and_bidirectional_streams() {
        use semantic_rpc::interface::InvocationOutput;
        use semantic_rpc::stream_command::{TypedEvent, TypedStream};
        let fixture = InterfaceFixture::start(|server| server, "").await;
        let output = fixture
            .invoke(
                "test.stream.sum",
                vec![payload(Value::Null), stream_argument(1..=100u32)],
            )
            .await
            .unwrap();
        let InvocationOutput::Values(values) = output else {
            panic!("expected values");
        };
        assert_eq!(values, vec![Value::U64(5050)]);

        let input = TypedStream::<String, u64>::from_events(futures_util::stream::iter([
            Ok(TypedEvent::Item("a".to_owned())),
            Ok(TypedEvent::Item("b".to_owned())),
            Ok(TypedEvent::End(7)),
        ]));
        let InvocationOutput::Stream(output) = fixture
            .invoke(
                "test.stream.echo",
                vec![
                    payload(Value::Null),
                    semantic_rpc::interface::InvocationArgument::Stream(input.into_owned()),
                ],
            )
            .await
            .unwrap()
        else {
            panic!("expected stream");
        };
        let events: Vec<_> = TypedStream::<String, u64>::from_owned(output)
            .collect()
            .await;
        assert_eq!(
            events,
            vec![
                Ok(TypedEvent::Item("a".to_owned())),
                Ok(TypedEvent::Item("b".to_owned())),
                Ok(TypedEvent::End(7)),
            ]
        );
        fixture.stop().await;
    }

    #[tokio::test]
    async fn command_export_runs_unary_commands_in_one_session_scope() {
        use semantic_rpc::interface::InvocationOutput;
        let fixture = InterfaceFixture::start(|server| server, "?scope=query").await;
        let current = |output: InvocationOutput| {
            let InvocationOutput::Values(values) = output else {
                panic!("expected values");
            };
            let [Value::Object(object)] = values.as_slice() else {
                panic!("expected one object, got {values:?}");
            };
            object.get("scope_id").cloned()
        };

        let output = fixture
            .invoke("semantic.scope.current", vec![payload(Value::Null)])
            .await
            .unwrap();
        assert_eq!(current(output), Some(Value::String("query".into())));

        fixture
            .invoke(
                "semantic.scope.use",
                vec![payload(Value::String("header".into()))],
            )
            .await
            .unwrap();
        let output = fixture
            .invoke("semantic.scope.current", vec![payload(Value::Void)])
            .await
            .unwrap();
        assert_eq!(current(output), Some(Value::String("header".into())));
        fixture.stop().await;
    }

    #[tokio::test]
    async fn typed_invoke_stream_covers_each_stream_shape() {
        use semantic_rpc::stream_command::{TypedEvent, TypedStream};
        let fixture = InterfaceFixture::start(|server| server, "").await;

        let stream = fixture
            .client
            .invoke_stream::<StreamCount>(3, ())
            .await
            .unwrap();
        let events: Vec<_> = stream.collect().await;
        assert_eq!(
            events,
            vec![
                Ok(TypedEvent::Item(0)),
                Ok(TypedEvent::Item(1)),
                Ok(TypedEvent::Item(2)),
                Ok(TypedEvent::End("done".to_owned())),
            ]
        );

        let input = TypedStream::<u32>::from_events(futures_util::stream::iter([
            Ok(TypedEvent::Item(20)),
            Ok(TypedEvent::Item(22)),
            Ok(TypedEvent::End(())),
        ]));
        let total = fixture
            .client
            .invoke_stream::<StreamSum>((), input)
            .await
            .unwrap();
        assert_eq!(total, 42);

        let input = TypedStream::<String, u64>::from_events(futures_util::stream::iter([
            Ok(TypedEvent::Item("x".to_owned())),
            Ok(TypedEvent::End(5)),
        ]));
        let echoed = fixture
            .client
            .invoke_stream::<StreamEcho>((), input)
            .await
            .unwrap();
        let events: Vec<_> = echoed.collect().await;
        assert_eq!(
            events,
            vec![Ok(TypedEvent::Item("x".to_owned())), Ok(TypedEvent::End(5)),]
        );
        fixture.stop().await;
    }

    #[tokio::test]
    async fn typed_invoke_stream_reports_remote_errors() {
        struct Missing;
        impl semantic_rpc::stream_command::RpcStreamCommandSpec for Missing {
            type Payload = ();
            type Input = ();
            type Output = semantic_data::value::StreamOf<u32>;
            type Error = semantic_app::AppError;

            const NAME: &'static str = "test.stream.missing";
        }

        let fixture = InterfaceFixture::start(|server| server, "").await;
        let error = match fixture.client.invoke_stream::<Missing>((), ()).await {
            Ok(_) => panic!("unknown command must fail"),
            Err(error) => error,
        };
        assert!(
            matches!(&error, semantic_rpc_core::RpcClientError::Remote(code, _) if code == "unknown_command"),
            "{error:?}"
        );
        fixture.stop().await;
    }

    #[tokio::test]
    async fn command_export_reports_unknown_commands_and_invalid_payloads() {
        let fixture = InterfaceFixture::start(|server| server, "").await;
        let error = fixture
            .invoke("test.missing", vec![payload(Value::Null)])
            .await
            .err()
            .unwrap();
        assert_eq!(error.code, "unknown_command");

        let error = fixture
            .invoke(
                "test.stream.count",
                vec![payload(Value::String("x".into()))],
            )
            .await
            .err()
            .unwrap();
        assert_eq!(error.code, "invalid_argument");

        let error = fixture
            .invoke("test.stream.sum", vec![payload(Value::Null)])
            .await
            .err()
            .unwrap();
        assert_eq!(error.code, "invalid_argument");
        fixture.stop().await;
    }

    #[tokio::test]
    async fn interface_websocket_requires_authentication() {
        let fixture = InterfaceFixture::start(
            |server| {
                server.with_principal_resolver(HeaderPrincipalResolver::new(
                    http::HeaderName::from_static("x-test-user"),
                ))
            },
            "",
        )
        .await;
        let error = fixture
            .invoke("semantic.scope.current", vec![payload(Value::Null)])
            .await
            .err()
            .unwrap();
        assert_eq!(error.code, "connection_lost");
        assert!(error.message.contains("401"), "{}", error.message);
        fixture.stop().await;
    }

    fn value_object(fields: impl IntoIterator<Item = (&'static str, Value)>) -> Value {
        let mut object = Object::new();
        for (key, value) in fields {
            object.insert(key, value);
        }
        Value::Object(object)
    }

    async fn post_rpc(
        server: &SemanticServer,
        uri: &str,
        scope_header: Option<&str>,
        command: &str,
        payload: Value,
    ) -> RpcResponse {
        let body = serde_json::to_vec(&RpcRequest {
            id: 11,
            command: command.to_string(),
            payload,
        })
        .unwrap();
        let mut builder = Request::builder()
            .method("POST")
            .uri(uri)
            .header("content-type", "application/json");
        if let Some(scope_header) = scope_header {
            builder = builder.header("x-semantic-scope", scope_header);
        }
        let response = server
            .router()
            .oneshot(builder.body(Body::from(body)).unwrap())
            .await
            .unwrap();
        assert!(response.status().is_success());
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice::<RpcResponse>(&bytes).unwrap()
    }

    async fn post_command(
        server: &SemanticServer,
        uri: &str,
        body: Vec<u8>,
    ) -> (http::StatusCode, RpcResult) {
        let response = server
            .router()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(uri)
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let result = match status {
            http::StatusCode::OK => RpcResult::Ok(
                serde_json::from_slice::<semantic_data::value::serde::typed::TypedValue>(&bytes)
                    .unwrap()
                    .0,
            ),
            _ => RpcResult::Err(serde_json::from_slice(&bytes).unwrap()),
        };
        (status, result)
    }

    fn select_db_name(response: RpcResponse) -> String {
        select_db_name_from_result(response.result)
    }

    fn select_db_name_from_result(result: RpcResult) -> String {
        let RpcResult::Ok(Value::Object(object)) = result else {
            panic!("expected ok object");
        };
        let Some(Value::List(rows)) = object.get("rows") else {
            panic!("expected rows");
        };
        let Some(Value::Object(row)) = rows.first() else {
            panic!("expected row");
        };
        let Some(Value::String(db)) = row.get("db") else {
            panic!("expected db string");
        };
        db.clone()
    }

    #[tokio::test]
    async fn http_rpc_uses_system_principal_by_default() {
        let server = SemanticServer::new(test_app());
        let response = post_rpc(
            &server,
            "/api/v1/rpc",
            None,
            "semantic.scope.current",
            Value::Void,
        )
        .await;
        let RpcResult::Ok(Value::Object(object)) = response.result else {
            panic!("expected ok object");
        };
        assert_eq!(object.get("scope_id"), Some(&Value::Null));
    }

    #[tokio::test]
    async fn http_scope_header_is_used() {
        let server = SemanticServer::new(test_app());
        let response = post_rpc(
            &server,
            "/api/v1/rpc",
            Some("header"),
            "semantic.db.query",
            value_object([("query", Value::String("select * from _".to_string()))]),
        )
        .await;
        assert_eq!(select_db_name(response), "header");
    }

    #[tokio::test]
    async fn query_param_scope_fallback_is_used() {
        let server = SemanticServer::new(test_app());
        let response = post_rpc(
            &server,
            "/api/v1/rpc?scope=query",
            None,
            "semantic.db.query",
            value_object([("query", Value::String("select * from _".to_string()))]),
        )
        .await;
        assert_eq!(select_db_name(response), "query");
    }

    #[tokio::test]
    async fn path_command_invokes_with_body_payload() {
        let server = SemanticServer::new(test_app());
        let payload = value_object([("query", Value::String("select * from _".to_string()))]);
        let body =
            serde_json::to_vec(&semantic_data::value::serde::typed::TypedRef(&payload)).unwrap();
        let (status, result) =
            post_command(&server, "/api/v1/rpc/semantic.db.query?scope=query", body).await;
        assert_eq!(status, http::StatusCode::OK);
        assert_eq!(select_db_name_from_result(result), "query");
    }

    #[tokio::test]
    async fn streaming_commands_require_a_streaming_session() {
        let server = SemanticServer::new(test_app());
        let response = post_rpc(
            &server,
            "/api/v1/rpc",
            None,
            "test.stream.echo",
            Value::Void,
        )
        .await;
        assert!(matches!(
            response.result,
            RpcResult::Err(err) if err.code == "streaming_required"
        ));

        let (status, result) = post_command(&server, "/api/v1/rpc/test.stream.echo", vec![]).await;
        assert_eq!(status, http::StatusCode::BAD_REQUEST);
        assert!(matches!(result, RpcResult::Err(err) if err.code == "streaming_required"));
    }

    #[tokio::test]
    async fn path_command_empty_body_is_void_payload() {
        let server = SemanticServer::new(test_app());
        let (status, result) =
            post_command(&server, "/api/v1/rpc/semantic.scope.current", vec![]).await;
        assert_eq!(status, http::StatusCode::OK);
        let RpcResult::Ok(Value::Object(object)) = result else {
            panic!("expected ok object");
        };
        assert_eq!(object.get("scope_id"), Some(&Value::Null));
    }

    #[tokio::test]
    async fn path_command_errors_map_to_http_statuses() {
        let server = SemanticServer::new(test_app());
        let (status, result) = post_command(&server, "/api/v1/rpc/semantic.missing", vec![]).await;
        assert_eq!(status, http::StatusCode::NOT_FOUND);
        assert!(matches!(result, RpcResult::Err(err) if err.code == "unknown_command"));

        let (status, result) = post_command(
            &server,
            "/api/v1/rpc/semantic.scope.current",
            b"{not json".to_vec(),
        )
        .await;
        assert_eq!(status, http::StatusCode::BAD_REQUEST);
        assert!(matches!(result, RpcResult::Err(err) if err.code == "invalid_payload"));

        let body = serde_json::to_vec(&semantic_data::value::serde::typed::TypedRef(&Value::U8(1)))
            .unwrap();
        let (status, result) =
            post_command(&server, "/api/v1/rpc/semantic.db.query?scope=query", body).await;
        assert_eq!(status, http::StatusCode::BAD_REQUEST);
        assert!(matches!(result, RpcResult::Err(err) if err.code == "invalid_payload"));

        let payload = value_object([("query", Value::String("select * from _".to_string()))]);
        let body =
            serde_json::to_vec(&semantic_data::value::serde::typed::TypedRef(&payload)).unwrap();
        let (status, result) =
            post_command(&server, "/api/v1/rpc/semantic.db.query?scope=missing", body).await;
        assert_eq!(status, http::StatusCode::NOT_FOUND);
        assert!(matches!(result, RpcResult::Err(err) if err.code == "unknown_scope"));
    }

    #[tokio::test]
    async fn path_command_introspection() {
        let server = SemanticServer::new(test_app());
        let (status, result) =
            post_command(&server, "/api/v1/rpc/semantic.command.list", vec![]).await;
        assert_eq!(status, http::StatusCode::OK);
        assert!(matches!(result, RpcResult::Ok(Value::Object(_))));

        let payload = value_object([("name", Value::String("semantic.missing".to_string()))]);
        let body =
            serde_json::to_vec(&semantic_data::value::serde::typed::TypedRef(&payload)).unwrap();
        let (status, result) =
            post_command(&server, "/api/v1/rpc/semantic.command.get", body).await;
        assert_eq!(status, http::StatusCode::NOT_FOUND);
        assert!(matches!(result, RpcResult::Err(err) if err.code == "unknown_command"));
    }

    #[tokio::test]
    async fn invalid_rpc_payload_returns_rpc_error() {
        let server = SemanticServer::new(test_app());
        let response = post_rpc(
            &server,
            "/api/v1/rpc",
            None,
            "semantic.db.query",
            Value::Void,
        )
        .await;
        let RpcResult::Err(err) = response.result else {
            panic!("expected rpc error");
        };
        assert_eq!(err.code, "invalid_payload");
        assert_eq!(err.message, "payload: expected object, found void");
    }

    #[tokio::test]
    async fn file_upload_and_download_round_trip() {
        let server = SemanticServer::new(test_app());
        let response = server
            .router()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/file")
                    .header("x-semantic-scope", "default")
                    .header("content-type", "text/plain")
                    .header("x-semantic-filename", "hello.txt")
                    .header(
                        "x-semantic-file-entity",
                        r#"{"semantic:title":"Hello","filename":"wrong.txt","filestore_locator":"wrong"}"#,
                    )
                    .body(Body::from("hello"))
                    .unwrap(),
            )
            .await
            .unwrap();
        if response.status() != http::StatusCode::CREATED {
            let status = response.status();
            let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
            panic!(
                "expected 201, got {status}: {}",
                String::from_utf8_lossy(&body)
            );
        }
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let Value::Object(upload) = serde_json::from_slice::<Value>(&bytes).unwrap() else {
            panic!("expected upload object");
        };
        let Some(Value::String(id)) = upload.get("id") else {
            panic!("expected file id");
        };
        let Some(Value::Object(object)) = upload.get("object") else {
            panic!("expected file object");
        };
        let locator = object
            .get("filestore_locator")
            .and_then(Value::as_str)
            .unwrap();
        let (hash_id, publication_id) = locator.split_once('/').unwrap();
        assert_eq!(hash_id, id);
        assert_eq!(
            publication_id.split('-').map(str::len).collect::<Vec<_>>(),
            [8, 4, 4, 4, 12]
        );
        assert!(
            publication_id
                .bytes()
                .all(|byte| byte == b'-' || byte.is_ascii_hexdigit())
        );
        assert_eq!(
            object.get("semantic:title").and_then(Value::as_str),
            Some("Hello")
        );
        assert_eq!(
            object.get("filename").and_then(Value::as_str),
            Some("hello.txt")
        );
        assert_eq!(
            object.get("mime_type").and_then(Value::as_str),
            Some("text/plain")
        );
        assert_eq!(object.get("filekind").and_then(Value::as_str), Some("text"));

        let response = server
            .router()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/api/v1/file/{id}"))
                    .header("x-semantic-scope", "default")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), http::StatusCode::OK);
        assert_eq!(
            response.headers().get(http::header::CONTENT_TYPE).unwrap(),
            "text/plain"
        );
        assert_eq!(
            response.headers().get(http::header::ACCEPT_RANGES).unwrap(),
            "bytes"
        );
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(&bytes[..], b"hello");

        let response = server
            .router()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/api/v1/file/{id}"))
                    .header("x-semantic-scope", "missing")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), http::StatusCode::NOT_FOUND);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let error: semantic_rpc_core::RpcError = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(error.code, "unknown_scope");
    }

    #[tokio::test]
    async fn file_upload_streams_bodies_larger_than_axums_default_limit() {
        const CHUNK_SIZE: usize = 64 * 1024;
        const CHUNK_COUNT: usize = 48;
        const TOTAL_SIZE: usize = CHUNK_SIZE * CHUNK_COUNT;

        let chunks = (0..CHUNK_COUNT)
            .map(|_| Ok::<_, std::convert::Infallible>(bytes::Bytes::from(vec![0x5a; CHUNK_SIZE])));
        let server = SemanticServer::new(test_app());
        let response = server
            .router()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/file")
                    .header("x-semantic-scope", "default")
                    .header("content-length", TOTAL_SIZE)
                    .body(Body::from_stream(futures_util::stream::iter(chunks)))
                    .unwrap(),
            )
            .await
            .unwrap();
        if response.status() != http::StatusCode::CREATED {
            let status = response.status();
            let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
            panic!(
                "expected 201, got {status}: {}",
                String::from_utf8_lossy(&body)
            );
        }
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let Value::Object(upload) = serde_json::from_slice::<Value>(&bytes).unwrap() else {
            panic!("expected upload object");
        };
        let Some(Value::Object(object)) = upload.get("object") else {
            panic!("expected file object");
        };
        assert_eq!(
            object.get("byte_size"),
            Some(&Value::U64(TOTAL_SIZE as u64))
        );
    }

    #[tokio::test]
    async fn file_upload_enforces_configured_streaming_limit() {
        let mut config = ServerConfig::default();
        config.max_file_upload_size = 4;
        let server = SemanticServer::new(test_app()).with_config(config);
        let response = server
            .router()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/file")
                    .header("x-semantic-scope", "default")
                    .header("content-length", 5)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), http::StatusCode::PAYLOAD_TOO_LARGE);

        let chunks = [
            Ok::<_, std::convert::Infallible>(bytes::Bytes::from_static(b"1234")),
            Ok(bytes::Bytes::from_static(b"5")),
        ];
        let response = server
            .router()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/file")
                    .header("x-semantic-scope", "default")
                    .body(Body::from_stream(futures_util::stream::iter(chunks)))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), http::StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[tokio::test]
    async fn file_download_supports_byte_ranges() {
        let server = SemanticServer::new(test_app());
        let response = server
            .router()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/file")
                    .header("x-semantic-scope", "default")
                    .body(Body::from("abcdef"))
                    .unwrap(),
            )
            .await
            .unwrap();
        if response.status() != http::StatusCode::CREATED {
            let status = response.status();
            let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
            panic!(
                "expected 201, got {status}: {}",
                String::from_utf8_lossy(&body)
            );
        }
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let Value::Object(upload) = serde_json::from_slice::<Value>(&bytes).unwrap() else {
            panic!("expected upload object");
        };
        let Some(Value::String(id)) = upload.get("id") else {
            panic!("expected file id");
        };

        let response = server
            .router()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/api/v1/file/{id}"))
                    .header("x-semantic-scope", "default")
                    .header("range", "bytes=1-3")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), http::StatusCode::PARTIAL_CONTENT);
        assert_eq!(
            response.headers().get(http::header::CONTENT_RANGE).unwrap(),
            "bytes 1-3/6"
        );
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(&bytes[..], b"bcd");

        let response = server
            .router()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/api/v1/file/{id}"))
                    .header("x-semantic-scope", "default")
                    .header("range", "bytes=3-")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), http::StatusCode::PARTIAL_CONTENT);
        assert_eq!(
            response.headers().get(http::header::CONTENT_RANGE).unwrap(),
            "bytes 3-5/6"
        );
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(&bytes[..], b"def");

        let response = server
            .router()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/api/v1/file/{id}"))
                    .header("x-semantic-scope", "default")
                    .header("range", "bytes=-2")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), http::StatusCode::PARTIAL_CONTENT);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(&bytes[..], b"ef");

        let response = server
            .router()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/api/v1/file/{id}"))
                    .header("x-semantic-scope", "default")
                    .header("range", "bytes=99-100")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), http::StatusCode::RANGE_NOT_SATISFIABLE);
    }
}
