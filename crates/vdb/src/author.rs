use std::{future::Future, pin::Pin, sync::Arc};

use async_trait::async_trait;
use futures_util::{Stream, StreamExt};
use semantic_data::{FromValue, IntoValue, Object, Value};
use semantic_plugin::{Plugin, PluginError, PluginInstanceContext, PluginManifest};
use semantic_rpc::interface::{
    CancellationToken, InterfaceImplementation, InvocationArgument, InvocationContext,
    InvocationOutput, OwnedValueStream, StreamEvent, ValidatedInvocation,
};
use semantic_rpc_core::interface::{ImplementationDescriptor, InvocationError};

use crate::{
    AcceptedScan, DatabaseDescriptor, FilterSupport, ScanPlan, ScanRequest, ScanSummary, VdbError,
};

pub type EntityStream = Pin<Box<dyn Stream<Item = Result<Object, VdbError>> + Send>>;

/// A read-only, polymorphic entity collection supplied by a plugin.
#[async_trait]
pub trait VirtualDatabase: Send + Sync + 'static {
    /// List only the schema exposed by this database. Bump `schema_revision`
    /// whenever those definitions change; the host never persists them.
    async fn describe(&self) -> Result<DatabaseDescriptor, VdbError>;

    /// Must match the revision that `describe` would currently return.
    /// The adapter copies it into every accepted negotiation result.
    fn schema_revision(&self) -> String;

    /// Default to a full scan: the host evaluates filters, ordering and limits.
    async fn negotiate(&self, request: &ScanRequest) -> Result<ScanPlan, VdbError> {
        Ok(ScanPlan::Accepted {
            plan: AcceptedScan {
                filters: vec![FilterSupport::Unsupported; request.filters.len()],
                ordered_prefix: 0,
                limit_applied: false,
                offset_applied: false,
                estimated_rows: None,
                token: None,
                schema_revision: self.schema_revision(),
            },
        })
    }

    /// Emit unique entity ids and honour the negotiated plan. Dropping the
    /// returned interface stream cancels this scan's token.
    fn scan(
        &self,
        request: ScanRequest,
        plan: AcceptedScan,
        bindings: Object,
        cancellation: CancellationToken,
    ) -> EntityStream;
}

/// Wrap an async instance factory as a Semantic plugin.
///
/// The factory receives scope configuration and cancellation context. Every
/// export in the manifest is served by the resulting virtual database.
pub struct VirtualDatabasePlugin<F> {
    manifest: PluginManifest,
    factory: F,
}

impl<F> VirtualDatabasePlugin<F> {
    pub fn new(manifest: PluginManifest, factory: F) -> Self {
        Self { manifest, factory }
    }
}

impl<F, Fut, V> Plugin for VirtualDatabasePlugin<F>
where
    F: Fn(PluginInstanceContext) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<V, VdbError>> + Send,
    V: VirtualDatabase,
{
    fn manifest(&self) -> &PluginManifest {
        &self.manifest
    }

    fn create<'a>(
        &'a self,
        context: PluginInstanceContext,
    ) -> Pin<
        Box<dyn Future<Output = Result<Arc<dyn InterfaceImplementation>, PluginError>> + Send + 'a>,
    > {
        Box::pin(async move {
            let database = (self.factory)(context)
                .await
                .map_err(|error| PluginError::new(error.code, error.message))?;
            Ok(Arc::new(VdbImplementation {
                exports: self.manifest.exports.clone(),
                database,
            }) as Arc<dyn InterfaceImplementation>)
        })
    }
}

struct VdbImplementation<V> {
    exports: Vec<ImplementationDescriptor>,
    database: V,
}

impl<V: VirtualDatabase> InterfaceImplementation for VdbImplementation<V> {
    fn descriptors(&self) -> &[ImplementationDescriptor] {
        &self.exports
    }

    fn invoke<'a>(
        &'a self,
        call: ValidatedInvocation,
        context: InvocationContext,
    ) -> Pin<Box<dyn Future<Output = Result<InvocationOutput, InvocationError>> + Send + 'a>> {
        Box::pin(async move {
            if !self
                .exports
                .iter()
                .any(|export| export.export == call.export)
            {
                return Err(InvocationError::new(
                    "unknown_export",
                    "unknown virtual database export",
                ));
            }
            let arity = match call.method.as_str() {
                "describe" => 0,
                "negotiate" => 1,
                "scan" => 3,
                _ => {
                    return Err(InvocationError::new(
                        "unknown_method",
                        "unknown virtual database method",
                    ));
                }
            };
            if call.arguments.len() != arity {
                return Err(InvocationError::new(
                    "invalid_input",
                    "wrong argument count",
                ));
            }
            let mut args = call.arguments.into_iter();
            match call.method.as_str() {
                "describe" => {
                    let descriptor = tokio::select! {
                        biased;
                        _ = context.cancellation.cancelled() => Err(cancelled()),
                        result = self.database.describe() => result.map_err(invocation_error),
                    }?;
                    Ok(InvocationOutput::Values(vec![descriptor.into_value()]))
                }
                "negotiate" => {
                    let request = decode(args.next())?;
                    let mut scan = tokio::select! {
                        biased;
                        _ = context.cancellation.cancelled() => Err(cancelled()),
                        result = self.database.negotiate(&request) => result.map_err(invocation_error),
                    }?;
                    if let ScanPlan::Accepted { plan } = &mut scan {
                        plan.schema_revision = self.database.schema_revision();
                        plan.validate(&request).map_err(invocation_error)?;
                    }
                    Ok(InvocationOutput::Values(vec![scan.into_value()]))
                }
                "scan" => {
                    let request = decode(args.next())?;
                    let plan: AcceptedScan = decode(args.next())?;
                    let bindings = decode(args.next())?;
                    plan.validate(&request).map_err(invocation_error)?;
                    let cancellation = context.cancellation.child_token();
                    let guard = CancelOnDrop(cancellation.clone());
                    let mut entities =
                        self.database
                            .scan(request, plan, bindings, cancellation.clone());
                    let stream = async_stream::try_stream! {
                        let _guard = guard;
                        let mut rows = 0u64;
                        loop {
                            let entity = tokio::select! {
                                biased;
                                _ = cancellation.cancelled() => Err(cancelled()),
                                entity = entities.next() => Ok(entity),
                            }?;
                            let Some(entity) = entity else { break };
                            let entity = entity.map_err(invocation_error)?;
                            rows = rows.checked_add(1).ok_or_else(|| InvocationError::new("protocol_violation", "row count overflow"))?;
                            yield StreamEvent::Item(Value::Object(entity));
                        }
                        yield StreamEvent::End(Some(ScanSummary { rows }.into_value()));
                    };
                    Ok(InvocationOutput::Stream(OwnedValueStream::new(stream)))
                }
                _ => unreachable!("method checked above"),
            }
        })
    }
}

struct CancelOnDrop(CancellationToken);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

fn decode<T: FromValue>(argument: Option<InvocationArgument>) -> Result<T, InvocationError> {
    let Some(InvocationArgument::Value(value)) = argument else {
        return Err(InvocationError::new(
            "invalid_input",
            "expected value argument",
        ));
    };
    T::from_value(value).map_err(|error| InvocationError::new("invalid_input", error.to_string()))
}

fn invocation_error(error: VdbError) -> InvocationError {
    let mut data = Object::new();
    data.insert("code", error.code.clone());
    data.insert("message", error.message.clone());
    InvocationError {
        code: error.code,
        message: error.message,
        data: Some(Value::Object(data)),
    }
}

fn cancelled() -> InvocationError {
    invocation_error(VdbError {
        code: "cancelled".into(),
        message: "virtual database invocation cancelled".into(),
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use semantic_data::query::{BinaryOp, Expr, Operand};
    use semantic_data::value::FieldPath;
    use semantic_db_core::catalog::Catalog;

    use super::*;
    use crate::{DatabaseSchema, implementation_descriptor};

    #[derive(Default, Clone)]
    struct TinyVdb {
        failure: bool,
        cancellation: Arc<Mutex<Option<CancellationToken>>>,
    }

    fn plugin_error() -> VdbError {
        VdbError {
            code: "tiny_failure".into(),
            message: "tiny database failed".into(),
        }
    }

    #[async_trait]
    impl VirtualDatabase for TinyVdb {
        async fn describe(&self) -> Result<DatabaseDescriptor, VdbError> {
            if self.failure {
                return Err(plugin_error());
            }
            Ok(DatabaseDescriptor {
                title: "Tiny database".into(),
                description: None,
                schema: DatabaseSchema::default(),
                schema_revision: self.schema_revision(),
                allow_untyped: true,
            })
        }

        fn schema_revision(&self) -> String {
            "7".into()
        }

        fn scan(
            &self,
            _request: ScanRequest,
            _plan: AcceptedScan,
            _bindings: Object,
            cancellation: CancellationToken,
        ) -> EntityStream {
            *self.cancellation.lock().unwrap() = Some(cancellation);
            if self.failure {
                return Box::pin(futures_util::stream::iter([Err(plugin_error())]));
            }
            Box::pin(futures_util::stream::iter(["one", "two", "three"].map(
                |id| {
                    let mut entity = Object::new();
                    entity.insert("id", Value::String(id.into()));
                    Ok(entity)
                },
            )))
        }
    }

    fn descriptor() -> ImplementationDescriptor {
        let mut catalog = Catalog::new();
        catalog.upsert_package(semantic_data::bundles::query::package());
        catalog.upsert_package(crate::package());
        implementation_descriptor(&catalog, "database").unwrap()
    }

    async fn instance(tiny: TinyVdb) -> Arc<dyn InterfaceImplementation> {
        let plugin = VirtualDatabasePlugin::new(
            PluginManifest {
                id: "tiny".into(),
                revision: "1".into(),
                title: "Tiny database".into(),
                exports: vec![descriptor()],
                configuration_schema: None,
                source_bindings: Default::default(),
            },
            move |_context| {
                let tiny = tiny.clone();
                async move { Ok(tiny) }
            },
        );
        assert_eq!(plugin.manifest().id, "tiny");
        plugin
            .create(PluginInstanceContext {
                scope: "test".into(),
                generation: 1,
                configuration: Value::Null,
                cancellation: CancellationToken::new(),
            })
            .await
            .unwrap()
    }

    fn call(method: &str, arguments: Vec<Value>) -> ValidatedInvocation {
        ValidatedInvocation {
            export: "database".into(),
            method: method.into(),
            arguments: arguments
                .into_iter()
                .map(InvocationArgument::Value)
                .collect(),
        }
    }

    fn request() -> ScanRequest {
        ScanRequest {
            filters: vec![Expr::Binary {
                op: BinaryOp::Eq,
                left: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                    "id",
                ])))),
                right: Box::new(Expr::Operand(Operand::Literal(Value::String("one".into())))),
            }],
            limit: Some(1),
            offset: 2,
            ..ScanRequest::default()
        }
    }

    async fn negotiate(
        instance: &dyn InterfaceImplementation,
        request: &ScanRequest,
    ) -> AcceptedScan {
        let InvocationOutput::Values(values) = instance
            .invoke(
                call("negotiate", vec![request.clone().into_value()]),
                InvocationContext::default(),
            )
            .await
            .unwrap()
        else {
            panic!("expected values")
        };
        let ScanPlan::Accepted { plan } =
            ScanPlan::from_value(values.into_iter().next().unwrap()).unwrap()
        else {
            panic!("expected accepted scan")
        };
        plan
    }

    #[tokio::test]
    async fn describe_default_negotiation_and_stream_summary() {
        let instance = instance(TinyVdb::default()).await;
        assert_eq!(instance.descriptors(), &[descriptor()]);
        let InvocationOutput::Values(values) = instance
            .invoke(call("describe", vec![]), InvocationContext::default())
            .await
            .unwrap()
        else {
            panic!("expected values")
        };
        let descriptor =
            DatabaseDescriptor::from_value(values.into_iter().next().unwrap()).unwrap();
        assert_eq!(descriptor.title, "Tiny database");
        assert!(descriptor.allow_untyped);
        let request = request();
        let plan = negotiate(&*instance, &request).await;
        assert_eq!(plan.filters, vec![FilterSupport::Unsupported]);
        assert_eq!(plan.schema_revision, "7");
        assert!(!plan.limit_applied && !plan.offset_applied);
        assert_eq!(plan.ordered_prefix, 0);
        let InvocationOutput::Stream(mut stream) = instance
            .invoke(
                call(
                    "scan",
                    vec![
                        request.into_value(),
                        plan.into_value(),
                        Object::new().into_value(),
                    ],
                ),
                InvocationContext::default(),
            )
            .await
            .unwrap()
        else {
            panic!("expected stream")
        };
        for id in ["one", "two", "three"] {
            let StreamEvent::Item(Value::Object(entity)) = stream.next().await.unwrap().unwrap()
            else {
                panic!("expected entity")
            };
            assert_eq!(entity.get("id"), Some(&Value::String(id.into())));
        }
        assert_eq!(
            stream.next().await.unwrap().unwrap(),
            StreamEvent::End(Some(ScanSummary { rows: 3 }.into_value()))
        );
        assert!(stream.next().await.is_none());
    }

    #[tokio::test]
    async fn invalid_calls_fail_before_dispatch() {
        let instance = instance(TinyVdb::default()).await;
        for (call, code) in [
            (call("describe", vec![Value::Null]), "invalid_input"),
            (call("negotiate", vec![]), "invalid_input"),
            (call("negotiate", vec![Value::Null]), "invalid_input"),
            (call("scan", vec![]), "invalid_input"),
            (call("unknown", vec![]), "unknown_method"),
            (
                ValidatedInvocation {
                    export: "missing".into(),
                    ..call("describe", vec![])
                },
                "unknown_export",
            ),
            (
                ValidatedInvocation {
                    export: "database".into(),
                    method: "negotiate".into(),
                    arguments: vec![InvocationArgument::Stream(OwnedValueStream::new(
                        futures_util::stream::empty(),
                    ))],
                },
                "invalid_input",
            ),
        ] {
            let Err(error) = instance.invoke(call, InvocationContext::default()).await else {
                panic!("expected error")
            };
            assert_eq!(error.code, code);
        }
    }

    #[tokio::test]
    async fn plugin_errors_preserve_code_message_and_throw_data() {
        let instance = instance(TinyVdb {
            failure: true,
            ..TinyVdb::default()
        })
        .await;
        let Err(error) = instance
            .invoke(call("describe", vec![]), InvocationContext::default())
            .await
        else {
            panic!("expected error")
        };
        assert_eq!(error.code, "tiny_failure");
        assert_eq!(
            VdbError::from_value(error.data.unwrap()).unwrap(),
            plugin_error()
        );
        let request = request();
        let plan = negotiate(&*instance, &request).await;
        let InvocationOutput::Stream(mut stream) = instance
            .invoke(
                call(
                    "scan",
                    vec![
                        request.into_value(),
                        plan.into_value(),
                        Object::new().into_value(),
                    ],
                ),
                InvocationContext::default(),
            )
            .await
            .unwrap()
        else {
            panic!("expected stream")
        };
        let error = stream.next().await.unwrap().unwrap_err();
        assert_eq!(error.code, "tiny_failure");
        assert_eq!(
            VdbError::from_value(error.data.unwrap()).unwrap(),
            plugin_error()
        );
        assert!(
            stream.next().await.is_none(),
            "a failed stream has no success summary"
        );
    }

    #[tokio::test]
    async fn cancellation_and_dropping_unpolled_stream_stop_scan() {
        let tiny = TinyVdb::default();
        let observed = tiny.cancellation.clone();
        let instance = instance(tiny).await;
        let request = request();
        let plan = negotiate(&*instance, &request).await;
        let context = InvocationContext::default();
        let parent = context.cancellation.clone();
        let InvocationOutput::Stream(stream) = instance
            .invoke(
                call(
                    "scan",
                    vec![
                        request.clone().into_value(),
                        plan.clone().into_value(),
                        Object::new().into_value(),
                    ],
                ),
                context,
            )
            .await
            .unwrap()
        else {
            panic!("expected stream")
        };
        let token = observed.lock().unwrap().clone().unwrap();
        assert!(!token.is_cancelled());
        drop(stream);
        assert!(token.is_cancelled());
        assert!(!parent.is_cancelled());

        let context = InvocationContext::default();
        let cancellation = context.cancellation.clone();
        let InvocationOutput::Stream(mut stream) = instance
            .invoke(
                call(
                    "scan",
                    vec![
                        request.into_value(),
                        plan.into_value(),
                        Object::new().into_value(),
                    ],
                ),
                context,
            )
            .await
            .unwrap()
        else {
            panic!("expected stream")
        };
        cancellation.cancel();
        assert_eq!(stream.next().await.unwrap().unwrap_err().code, "cancelled");
        assert!(stream.next().await.is_none());
        let context = InvocationContext::default();
        context.cancellation.cancel();
        let Err(error) = instance.invoke(call("describe", vec![]), context).await else {
            panic!("expected error")
        };
        assert_eq!(error.code, "cancelled");
    }

    #[test]
    fn missing_interface_package_is_an_author_error() {
        assert_eq!(
            implementation_descriptor(&Catalog::new(), "database")
                .unwrap_err()
                .code,
            "invalid_interface"
        );
        assert_eq!(descriptor().package_version, "1.0.0");
        assert!(descriptor().fingerprint.starts_with("v1:"));
    }

    struct OverrideVdb {
        malformed: bool,
    }

    #[async_trait]
    impl VirtualDatabase for OverrideVdb {
        async fn describe(&self) -> Result<DatabaseDescriptor, VdbError> {
            TinyVdb::default().describe().await
        }

        fn schema_revision(&self) -> String {
            "current".into()
        }

        async fn negotiate(&self, request: &ScanRequest) -> Result<ScanPlan, VdbError> {
            let ScanPlan::Accepted { mut plan } = TinyVdb::default().negotiate(request).await?
            else {
                unreachable!()
            };
            plan.schema_revision = "stale".into();
            if self.malformed {
                plan.filters.clear();
            }
            Ok(ScanPlan::Accepted { plan })
        }

        fn scan(
            &self,
            request: ScanRequest,
            plan: AcceptedScan,
            bindings: Object,
            cancellation: CancellationToken,
        ) -> EntityStream {
            TinyVdb::default().scan(request, plan, bindings, cancellation)
        }
    }

    #[tokio::test]
    async fn adapter_sets_revision_and_rejects_malformed_scan_plans() {
        let instance = VdbImplementation {
            exports: vec![descriptor()],
            database: OverrideVdb { malformed: false },
        };
        let request = request();
        let mut plan = negotiate(&instance, &request).await;
        assert_eq!(plan.schema_revision, "current");
        plan.filters.clear();
        let Err(error) = instance
            .invoke(
                call(
                    "scan",
                    vec![
                        request.clone().into_value(),
                        plan.into_value(),
                        Object::new().into_value(),
                    ],
                ),
                InvocationContext::default(),
            )
            .await
        else {
            panic!("expected error")
        };
        assert_eq!(error.code, "protocol_violation");
        let instance = VdbImplementation {
            exports: vec![descriptor()],
            database: OverrideVdb { malformed: true },
        };
        let Err(error) = instance
            .invoke(
                call("negotiate", vec![request.into_value()]),
                InvocationContext::default(),
            )
            .await
        else {
            panic!("expected error")
        };
        assert_eq!(error.code, "protocol_violation");
    }
}
