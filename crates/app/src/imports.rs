//! Application adapters for ordinary entity and File publication.
use crate::{AppError, FileService, FileSizedStream, SemanticDb};
use async_trait::async_trait;
use bytes::Bytes;
use futures_util::StreamExt;
use semantic_data::{
    Object,
    builtin::{ATTR_ID, ATTR_TYPE},
};
use semantic_db_core::{Batch, BatchOperation, DEFAULT_COLLECTION};
use semantic_import::*;
use std::sync::Arc;

fn error(error: impl std::fmt::Display) -> ImportError {
    ImportError::new("storage", error.to_string())
}

pub struct AppImportWriter {
    db: Arc<dyn SemanticDb>,
    store: Option<objstore::DynObjStore>,
    context: Option<(crate::AppRequestContext, crate::DbScopeId)>,
    files: FileService,
}
impl AppImportWriter {
    pub fn new(db: Arc<dyn SemanticDb>, store: objstore::DynObjStore, files: FileService) -> Self {
        Self {
            db,
            store: Some(store),
            files,
            context: None,
        }
    }
    fn for_context(
        db: Arc<dyn SemanticDb>,
        context: crate::AppRequestContext,
        scope: crate::DbScopeId,
    ) -> Self {
        Self {
            db,
            store: None,
            files: context.app.files().clone(),
            context: Some((context, scope)),
        }
    }
}
fn identity_object(identity: &SourceIdentity, key: &str, mut attributes: Object) -> Object {
    attributes.insert(SOURCE_NAMESPACE, identity.namespace.clone());
    attributes.insert(SOURCE_IDENTITY, identity.source.clone());
    attributes.insert(SOURCE_KEY, key.to_string());
    attributes
}
async fn publish(db: &dyn SemanticDb, id: String, object: Object) -> Result<(), ImportError> {
    db.execute_batch(Batch {
        operations: vec![BatchOperation::Upsert {
            collection: DEFAULT_COLLECTION.into(),
            id,
            object,
        }],
        ..Default::default()
    })
    .await
    .map_err(error)?;
    Ok(())
}
#[async_trait]
impl ImportWriter for AppImportWriter {
    async fn write_entity(
        &self,
        identity: &SourceIdentity,
        entity: EntityProposal,
    ) -> Result<(), ImportError> {
        let id = uuid::Uuid::from(stable_entity_id(
            &identity.namespace,
            &identity.source,
            &entity.key,
        ))
        .to_string();
        let catalog = self.db.catalog().await.map_err(error)?;
        let strict = catalog
            .class_ids(&entity.class)
            .first()
            .and_then(|id| catalog.class_by_lid(*id))
            .is_some_and(|class| class.class.strict_schema);
        // Provenance is optional ordinary metadata. Do not make an otherwise
        // valid strict class unpublishable by adding undeclared attributes.
        let mut object = if strict {
            entity.attributes
        } else {
            identity_object(identity, &entity.key, entity.attributes)
        };
        object.insert(ATTR_ID, id.clone());
        object.insert(ATTR_TYPE, entity.class);
        publish(self.db.as_ref(), id, object).await
    }
    async fn begin_file(
        &self,
        identity: &SourceIdentity,
        metadata: &FileMetadata,
    ) -> Result<Box<dyn FileWrite>, ImportError> {
        let id = uuid::Uuid::from(stable_entity_id(
            &identity.namespace,
            &identity.source,
            &metadata.key,
        ))
        .to_string();
        let object = identity_object(identity, &metadata.key, metadata.attributes.clone());
        let store = match &self.store {
            Some(store) => store.clone(),
            None => {
                let (context, scope) = self.context.as_ref().expect("writer store source");
                context
                    .default_file_store(Some(scope.clone()))
                    .await
                    .map_err(error)?
            }
        };
        let files = self.files.clone();
        let filename = metadata.filename.clone();
        let mime = metadata.mime_type.clone();
        let size = metadata.expected_size;
        let (sender, receiver) = tokio::sync::mpsc::channel::<Result<Bytes, AppError>>(1);
        let complete = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stream_complete = complete.clone();
        let stream =
            futures_util::stream::unfold((receiver, false), move |(mut receiver, ended)| {
                let complete = stream_complete.clone();
                async move {
                    if ended {
                        return None;
                    }
                    match receiver.recv().await {
                        Some(chunk) => Some((chunk, (receiver, false))),
                        None if complete.load(std::sync::atomic::Ordering::Acquire) => None,
                        None => Some((
                            Err(AppError::InvalidRequest("incomplete imported file".into())),
                            (receiver, true),
                        )),
                    }
                }
            })
            .boxed();
        let prepare_id = id.clone();
        let task = tokio::spawn(async move {
            files
                .prepare_import(
                    store.as_ref(),
                    prepare_id,
                    filename,
                    mime,
                    object,
                    FileSizedStream { stream, size },
                )
                .await
        });
        Ok(Box::new(AppFileWrite {
            db: self.db.clone(),
            id,
            sender: Some(sender),
            task: Some(task),
            complete,
        }))
    }
}
struct AppFileWrite {
    db: Arc<dyn SemanticDb>,
    id: String,
    sender: Option<tokio::sync::mpsc::Sender<Result<Bytes, AppError>>>,
    task: Option<tokio::task::JoinHandle<Result<Object, AppError>>>,
    complete: Arc<std::sync::atomic::AtomicBool>,
}
#[async_trait]
impl FileWrite for AppFileWrite {
    async fn write_chunk(&mut self, bytes: Bytes) -> Result<(), ImportError> {
        self.sender
            .as_ref()
            .ok_or_else(|| error("file upload closed"))?
            .send(Ok(bytes))
            .await
            .map_err(|_| error("file upload failed"))
    }
    async fn finish(mut self: Box<Self>) -> Result<(), ImportError> {
        self.complete
            .store(true, std::sync::atomic::Ordering::Release);
        self.sender.take();
        let object = self
            .task
            .take()
            .ok_or_else(|| error("file upload closed"))?
            .await
            .map_err(error)?
            .map_err(error)?;
        publish(self.db.as_ref(), self.id.clone(), object).await
    }
    async fn abort(mut self: Box<Self>) {
        self.sender.take();
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}
// Dropping the sender makes the upload fail and clean its owned temporary object.
// The task remains alive to settle its store write rather than abandoning it.
impl Drop for AppFileWrite {
    fn drop(&mut self) {
        self.sender.take();
    }
}

impl crate::AppRequestContext {
    pub async fn import_candidates(
        &self,
        scope: Option<crate::DbScopeId>,
        request: &SourceRequest,
        operation: Operation,
    ) -> Result<Vec<SourceCandidate>, AppError> {
        let scope = self.resolve_scope_id(scope).await?;
        let plugins = self.app.plugins(&self.principal, scope).await?;
        let bindings = plugins.runtime.bindings().await;
        let mut sources = Vec::new();
        let mut unavailable = Vec::new();
        for source in bindings.iter().filter(|b| {
            b.descriptor().interface.package == PACKAGE_NAME
                && b.descriptor().interface.name == "Source"
        }) {
            let interface = if operation == Operation::Fetch {
                "Fetcher"
            } else {
                "Importer"
            };
            let operations: Vec<_> = bindings
                .iter()
                .filter(|b| {
                    b.plugin_id() == source.plugin_id()
                        && b.generation() == source.generation()
                        && b.descriptor().interface.package == PACKAGE_NAME
                        && b.descriptor().interface.name == interface
                        && b.source_export() == Some(source.descriptor().export.as_str())
                })
                .collect();
            if operations.is_empty() {
                continue;
            }
            let described = match source.invoke("describe", Vec::new()).await {
                Ok(semantic_rpc::interface::InvocationOutput::Values(mut values))
                    if values.len() == 1 =>
                {
                    SourceDescriptor::from_value(values.remove(0))
                }
                Ok(_) => Err(ImportError::new(
                    "invalid_output",
                    "invalid source descriptor",
                )),
                Err(error) => Err(ImportError::new(error.code, error.message)),
            };
            for binding in operations {
                match &described {
                    Ok(descriptor) => {
                        sources.push((source.clone(), binding.clone(), descriptor.clone()))
                    }
                    Err(error) => unavailable.push(SourceCandidate {
                        source: source.clone(),
                        operation: binding.clone(),
                        priority: source.priority().unwrap_or(0),
                        descriptor: SourceDescriptor {
                            namespace: source.plugin_id().into(),
                            title: source.plugin_id().into(),
                            operations: vec![operation],
                            input_kinds: Vec::new(),
                            url_schemes: Vec::new(),
                            default_priority: 0,
                        },
                        probe: ProbeResult::Unavailable(error.to_string()),
                    }),
                }
            }
        }
        let mut candidates = discover(sources, request, operation).await;
        candidates.extend(unavailable);
        candidates.sort_by(|a, b| {
            b.priority
                .cmp(&a.priority)
                .then_with(|| a.operation.plugin_id().cmp(b.operation.plugin_id()))
                .then_with(|| {
                    a.operation
                        .descriptor()
                        .export
                        .cmp(&b.operation.descriptor().export)
                })
        });
        Ok(candidates)
    }
    pub async fn fetch_source(
        &self,
        scope: Option<crate::DbScopeId>,
        request: SourceRequest,
        explicit: Option<(&str, &str)>,
    ) -> Result<FetchedContent, AppError> {
        self.fetch_source_checked(scope, request, explicit, None)
            .await
    }
    pub async fn fetch_source_checked(
        &self,
        scope: Option<crate::DbScopeId>,
        request: SourceRequest,
        explicit: Option<(&str, &str)>,
        expected_generation: Option<u64>,
    ) -> Result<FetchedContent, AppError> {
        let candidates = self
            .import_candidates(scope, &request, Operation::Fetch)
            .await?;
        let binding = select(&candidates, explicit).map_err(crate::plugins::error)?;
        if expected_generation.is_some_and(|generation| generation != binding.generation()) {
            return Err(crate::plugins::error(
                "plugin_changed: selected generation is stale",
            ));
        }
        let candidate = candidates
            .iter()
            .find(|c| {
                c.operation.plugin_id() == binding.plugin_id()
                    && c.operation.descriptor().export == binding.descriptor().export
            })
            .expect("selected candidate");
        let identity = SourceIdentity {
            namespace: candidate.descriptor.namespace.clone(),
            source: url::Url::parse(&request.url)
                .map_err(crate::plugins::error)?
                .to_string(),
        };
        fetch(&binding, identity, request)
            .await
            .map_err(crate::plugins::error)
    }
    pub async fn start_import_source(
        &self,
        scope: Option<crate::DbScopeId>,
        request: SourceRequest,
        explicit: Option<(&str, &str)>,
    ) -> Result<semantic_jobs::JobTicket<ContentSummary>, AppError> {
        self.start_import_source_checked(scope, request, explicit, None)
            .await
    }
    pub async fn start_import_source_checked(
        &self,
        scope: Option<crate::DbScopeId>,
        request: SourceRequest,
        explicit: Option<(&str, &str)>,
        expected_generation: Option<u64>,
    ) -> Result<semantic_jobs::JobTicket<ContentSummary>, AppError> {
        let scope = self.resolve_scope_id(scope).await?;
        let candidates = self
            .import_candidates(Some(scope.clone()), &request, Operation::ImportSource)
            .await?;
        let binding = select(&candidates, explicit).map_err(crate::plugins::error)?;
        if expected_generation.is_some_and(|generation| generation != binding.generation()) {
            return Err(crate::plugins::error(
                "plugin_changed: selected generation is stale",
            ));
        }
        let candidate = candidates
            .iter()
            .find(|c| {
                c.operation.plugin_id() == binding.plugin_id()
                    && c.operation.descriptor().export == binding.descriptor().export
            })
            .expect("selected candidate");
        let identity = SourceIdentity {
            namespace: candidate.descriptor.namespace.clone(),
            source: url::Url::parse(&request.url)
                .map_err(crate::plugins::error)?
                .to_string(),
        };
        let writer = Arc::new(AppImportWriter::for_context(
            self.resolve_db(Some(scope.clone())).await?,
            self.clone(),
            scope.clone(),
        ));
        let groups = vec![binding.group()];
        self.jobs(Some(scope))
            .await?
            .submit(
                self.app.import_registration(),
                ImportJobInput {
                    binding,
                    identity,
                    input: ImportInput::Source(request),
                    writer,
                },
                semantic_jobs::SubmitOptions { groups },
            )
            .await
            .map_err(crate::plugins::error)
    }
    pub async fn start_import_fetched(
        &self,
        scope: Option<crate::DbScopeId>,
        binding: semantic_plugin::PluginBinding,
        request: FetchedRequest,
        fetched: FetchedContent,
    ) -> Result<semantic_jobs::JobTicket<ContentSummary>, AppError> {
        let scope = self.resolve_scope_id(scope).await?;
        let mut groups = fetched.groups;
        groups.push(binding.group());
        let writer = Arc::new(AppImportWriter::for_context(
            self.resolve_db(Some(scope.clone())).await?,
            self.clone(),
            scope.clone(),
        ));
        self.jobs(Some(scope))
            .await?
            .submit(
                self.app.import_registration(),
                ImportJobInput {
                    binding,
                    identity: request.identity.clone(),
                    input: ImportInput::Fetched {
                        request,
                        content: fetched.stream,
                    },
                    writer,
                },
                semantic_jobs::SubmitOptions { groups },
            )
            .await
            .map_err(crate::plugins::error)
    }
}
