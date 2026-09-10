//! Scope-aware streaming application adapter alongside the existing unary API.
use crate::{AppRequestContext, DbScopeId};
use semantic_data::import::{FetchedRequest, Operation, ProbeResult, SourceRequest};
use semantic_data::{Value, value::Object};
use semantic_db_core::catalog::Catalog;
use semantic_rpc::interface::{
    ConformingImplementation, ImplementationDescriptor, InterfaceImplementation, InterfaceRef,
    InvocationArgument, InvocationContext, InvocationError, InvocationFuture, InvocationOutput,
    ValidatedInvocation,
};
use std::{collections::BTreeMap, sync::Arc};

pub const APPLICATION_EXPORT: &str = "application";

pub fn implementation(
    context: AppRequestContext,
) -> Result<Arc<dyn InterfaceImplementation>, InvocationError> {
    let mut catalog = Catalog::new();
    catalog.upsert_package(semantic_data::import::package());
    let schema = catalog
        .resolve_interface("semantic.import", "v1", None, "Application")
        .map_err(error)?;
    let descriptor = ImplementationDescriptor {
        export: APPLICATION_EXPORT.into(),
        interface: InterfaceRef {
            package: "semantic.import".into(),
            module: "v1".into(),
            contract: None,
            name: "Application".into(),
        },
        package_version: "1.0.0".into(),
        fingerprint: schema.fingerprint,
    };
    let inner = Arc::new(ApplicationInterface {
        context,
        descriptors: vec![descriptor],
    });
    Ok(Arc::new(ConformingImplementation::new(
        inner,
        BTreeMap::from([(APPLICATION_EXPORT.into(), schema.interface)]),
        schema.definitions,
    )?))
}

struct ApplicationInterface {
    context: AppRequestContext,
    descriptors: Vec<ImplementationDescriptor>,
}

fn error(error: impl ToString) -> InvocationError {
    InvocationError::new("application_error", error.to_string())
}
fn invalid(message: &str) -> InvocationError {
    InvocationError::new("invalid_argument", message)
}
fn string(object: &mut Object, key: &str) -> Result<Option<String>, InvocationError> {
    match object.remove(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value)),
        _ => Err(invalid("expected string argument")),
    }
}

impl InterfaceImplementation for ApplicationInterface {
    fn descriptors(&self) -> &[ImplementationDescriptor] {
        &self.descriptors
    }
    fn invoke<'a>(
        &'a self,
        mut call: ValidatedInvocation,
        invocation: InvocationContext,
    ) -> InvocationFuture<'a> {
        Box::pin(async move {
            let cancellation = invocation.cancellation.clone();
            // Dropping a binding call signals its cooperative token; its runtime
            // task retains ownership until cleanup finishes. Submitted jobs are
            // likewise owned by the coordinator after admission.
            let work = async move {
                if invocation.cancellation.is_cancelled() {
                    return Err(InvocationError::new("cancelled", "call cancelled"));
                }
                if call.arguments.is_empty() {
                    return Err(invalid("missing request"));
                }
                let InvocationArgument::Value(Value::Object(mut envelope)) =
                    call.arguments.remove(0)
                else {
                    return Err(invalid("expected request object"));
                };
                let scope = string(&mut envelope, "scope_id")?.map(DbScopeId::new);
                let plugin = string(&mut envelope, "plugin_id")?;
                let export = string(&mut envelope, "export")?;
                let explicit = match (&plugin, &export) {
                    (Some(plugin), Some(export)) => Some((plugin.as_str(), export.as_str())),
                    (None, None) => None,
                    _ => return Err(invalid("plugin_id and export must be supplied together")),
                };
                let generation = match envelope.remove("generation") {
                    None | Some(Value::Null) => None,
                    Some(Value::U64(value)) => Some(value),
                    _ => return Err(invalid("expected generation u64")),
                };
                if call.method == "start_import_fetched" {
                    let (plugin, export) = explicit.ok_or_else(|| {
                        invalid("fetched import requires explicit plugin and export")
                    })?;
                    let resolved_scope = self
                        .context
                        .resolve_scope_id(scope.clone())
                        .await
                        .map_err(error)?;
                    let plugins = self
                        .context
                        .app
                        .plugins(&self.context.principal, resolved_scope)
                        .await
                        .map_err(error)?;
                    let binding = plugins
                        .runtime
                        .bindings()
                        .await
                        .into_iter()
                        .find(|binding| {
                            binding.plugin_id() == plugin && binding.descriptor().export == export
                        })
                        .ok_or_else(|| invalid("selected importer unavailable"))?;
                    if generation.is_some_and(|generation| generation != binding.generation()) {
                        return Err(InvocationError::new(
                            "plugin_changed",
                            "selected plugin generation changed",
                        ));
                    }
                    let request =
                        FetchedRequest::from_value(Value::Object(envelope)).map_err(error)?;
                    if call.arguments.len() != 1 {
                        return Err(invalid("expected owned content stream"));
                    }
                    let InvocationArgument::Stream(stream) = call.arguments.remove(0) else {
                        return Err(invalid("expected owned content stream"));
                    };
                    let groups = stream
                        .metadata::<Vec<semantic_jobs::JobGroup>>()
                        .cloned()
                        .unwrap_or_default();
                    let fetched = semantic_import::FetchedContent {
                        identity: request.identity.clone(),
                        stream,
                        groups,
                    };
                    let ticket = self
                        .context
                        .start_import_fetched(scope, binding, request, fetched)
                        .await
                        .map_err(error)?;
                    let mut result = Object::new();
                    result.insert("id", ticket.id.0);
                    return Ok(InvocationOutput::Values(vec![Value::Object(result)]));
                }
                let operation = string(&mut envelope, "operation")?
                    .map(|operation| Operation::parse(&operation))
                    .transpose()
                    .map_err(error)?
                    .unwrap_or(Operation::ImportSource);
                let request = SourceRequest::from_value(Value::Object(envelope)).map_err(error)?;
                match call.method.as_str() {
                    "list_candidates" => {
                        let candidates = self
                            .context
                            .import_candidates(scope, &request, operation)
                            .await
                            .map_err(error)?;
                        let values = candidates
                            .into_iter()
                            .map(|candidate| {
                                let mut value = Object::new();
                                value.insert(
                                    "plugin_id",
                                    candidate.operation.plugin_id().to_owned(),
                                );
                                value.insert(
                                    "export",
                                    candidate.operation.descriptor().export.clone(),
                                );
                                value.insert(
                                    "source_export",
                                    candidate.source.descriptor().export.clone(),
                                );
                                value.insert(
                                    "generation",
                                    Value::U64(candidate.operation.generation()),
                                );
                                value.insert("priority", Value::I32(candidate.priority));
                                value.insert("descriptor", candidate.descriptor.to_value());
                                let (status, reason) = match candidate.probe {
                                    ProbeResult::Supported => ("supported", None),
                                    ProbeResult::Unsupported(reason) => {
                                        ("unsupported", Some(reason))
                                    }
                                    ProbeResult::Unavailable(reason) => {
                                        ("unavailable", Some(reason))
                                    }
                                };
                                value.insert("status", status.to_owned());
                                value.insert(
                                    "reason",
                                    reason.map(Value::String).unwrap_or(Value::Null),
                                );
                                Value::Object(value)
                            })
                            .collect();
                        Ok(InvocationOutput::Values(vec![Value::List(values)]))
                    }
                    "fetch_source" => {
                        let fetched = self
                            .context
                            .fetch_source_checked(scope, request, explicit, generation)
                            .await
                            .map_err(error)?;
                        Ok(InvocationOutput::Stream(
                            fetched
                                .stream
                                .with_metadata(fetched.groups)
                                .with_cancellation(invocation.cancellation),
                        ))
                    }
                    "start_import_source" => {
                        let ticket = self
                            .context
                            .start_import_source_checked(scope, request, explicit, generation)
                            .await
                            .map_err(error)?;
                        let mut result = Object::new();
                        result.insert("id", ticket.id.0);
                        Ok(InvocationOutput::Values(vec![Value::Object(result)]))
                    }
                    _ => Err(invalid("unknown application interface method")),
                }
            };
            tokio::select! {
                biased;
                _ = cancellation.cancelled() => Err(InvocationError::new("cancelled", "call cancelled")),
                result = work => result,
            }
        })
    }
}
