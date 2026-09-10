//! Persistence adapter for scope-local plugin installations.
use crate::{AppError, SemanticDb};
use semantic_data::{
    Object, Value,
    builtin::{ATTR_ID, ATTR_TYPE},
    plugin::*,
    query::{FieldFormat, SelectQuery},
};
use semantic_db_core::{Batch, BatchOperation, QueryResult};
use semantic_plugin::{PluginRegistry, ScopePlugins};
use std::sync::Arc;

#[cfg(test)]
mod tests;

pub(crate) fn error(error: impl std::fmt::Display) -> AppError {
    AppError::InvalidRequest(error.to_string())
}

pub struct AppScopePlugins {
    pub runtime: ScopePlugins,
    db: Arc<dyn SemanticDb>,
    changes: tokio::sync::Mutex<()>,
    catalog: Arc<std::sync::RwLock<Arc<semantic_db_core::catalog::Catalog>>>,
}
impl AppScopePlugins {
    pub(crate) async fn update_package(
        self: &Arc<Self>,
        package: semantic_data::schema::Package,
    ) -> Result<semantic_db_core::PackageRegistrationOutcome, AppError> {
        let plugins = self.clone();
        // Once invalidation starts, caller cancellation must not strand live state
        // between the old catalog and the committed package definition.
        tokio::spawn(async move { plugins.update_package_inner(package).await })
            .await
            .map_err(error)?
    }

    async fn update_package_inner(
        &self,
        package: semantic_data::schema::Package,
    ) -> Result<semantic_db_core::PackageRegistrationOutcome, AppError> {
        let _guard = self.changes.lock().await;
        let before = self.db.catalog().await?;
        let mut after = before.as_ref().clone();
        after.upsert_package(package.clone());
        let affected: Vec<_> = self
            .list()
            .await?
            .into_iter()
            .filter(|a| {
                a.exports.iter().any(|e| {
                    if e.package == package.name {
                        return true;
                    }
                    let resolve = |catalog: &semantic_db_core::catalog::Catalog| {
                        catalog
                            .resolve_interface(
                                &e.package,
                                &e.module,
                                e.contract.as_deref(),
                                &e.interface,
                            )
                            .map(|resolved| (resolved.fingerprint, resolved.package_version))
                    };
                    match (resolve(&before), resolve(&after)) {
                        (Ok(before), Ok(after)) => before != after,
                        // An unresolved dependency must never retain a stale binding.
                        _ => true,
                    }
                })
            })
            .collect();
        // Validate generation allocation before performing any destructive work.
        for activation in &affected {
            activation
                .generation
                .checked_add(1)
                .ok_or_else(|| error("plugin generation exhausted"))?;
        }
        let mut stop_error = None;
        for activation in &affected {
            let mut disabled = activation.clone();
            disabled.enabled = false;
            if let Err(err) = self.runtime.activate(disabled).await {
                stop_error.get_or_insert_with(|| error(err));
            }
        }
        let outcome = match stop_error {
            Some(err) => Err(err),
            None => self
                .db
                .upsert_package(package)
                .await
                .map_err(AppError::from),
        };
        // Read the actual catalog even when registration failed: reconciliation is
        // against durable state, never the speculative definition above.
        let catalog = self.db.catalog().await;
        let mut reconciliation_errors = Vec::new();
        let catalog_ready = match catalog {
            Ok(catalog) => match self.catalog.write() {
                Ok(mut current) => {
                    *current = catalog;
                    true
                }
                Err(_) => {
                    reconciliation_errors.push("catalog lock poisoned".to_owned());
                    false
                }
            },
            Err(err) => {
                reconciliation_errors.push(err.to_string());
                false
            }
        };
        for mut activation in affected {
            activation.generation += 1;
            if let Err(err) = self.persist(&activation).await {
                reconciliation_errors.push(format!("{}: {err}", activation.id));
                continue;
            }
            if catalog_ready {
                // Conformance failures are represented by the plugin's observable
                // failed state. They do not undo a successfully installed package.
                let _ = self.runtime.activate(activation).await;
            }
        }
        if !reconciliation_errors.is_empty() {
            if let Err(err) = outcome {
                reconciliation_errors.insert(0, err.to_string());
            }
            return Err(error(reconciliation_errors.join("; ")));
        }
        outcome
    }
    #[cfg(test)]
    pub(crate) async fn open(
        scope: String,
        registry: PluginRegistry,
        jobs: semantic_jobs::ScopeJobs,
        db: Arc<dyn SemanticDb>,
    ) -> Result<Self, AppError> {
        Self::open_with_cancellation(
            scope,
            registry,
            jobs,
            db,
            semantic_jobs::CancellationToken::new(),
        )
        .await
    }
    pub(crate) async fn open_with_cancellation(
        scope: String,
        registry: PluginRegistry,
        jobs: semantic_jobs::ScopeJobs,
        db: Arc<dyn SemanticDb>,
        cancellation: semantic_jobs::CancellationToken,
    ) -> Result<Self, AppError> {
        let existing = db.catalog().await?;
        let expected = semantic_db_core::normalize_package_definition(&package()).map_err(error)?;
        if existing
            .package_by_name(semantic_data::plugin::PACKAGE_NAME)
            .is_some_and(|stored| stored != &expected)
        {
            return Err(error("incompatible stored semantic.plugin package"));
        }
        db.upsert_package(package()).await?;
        let defaults = registry.manifests();
        let catalog = Arc::new(std::sync::RwLock::new(db.catalog().await?));
        let validation_catalog = catalog.clone();
        let runtime = ScopePlugins::new(scope, registry, jobs)
            .with_cancellation(cancellation)
            .with_conformance(move |implementation| {
                let catalog = validation_catalog.read().map_err(|_| {
                    semantic_plugin::PluginError::new("catalog", "catalog lock poisoned")
                })?;
                let mut declarations = std::collections::BTreeMap::new();
                let mut definitions = std::collections::BTreeMap::new();
                for descriptor in implementation.descriptors() {
                    let reference = &descriptor.interface;
                    let resolved = catalog
                        .resolve_interface(
                            &reference.package,
                            &reference.module,
                            reference.contract.as_deref(),
                            &reference.name,
                        )
                        .map_err(|e| {
                            semantic_plugin::PluginError::new(
                                "interface_incompatible",
                                e.to_string(),
                            )
                        })?;
                    let version = resolved
                        .package_version
                        .as_ref()
                        .map(|v| {
                            format!(
                                "{}.{}.{}{}{}",
                                v.major,
                                v.minor,
                                v.patch,
                                v.pre.as_ref().map(|v| format!("-{v}")).unwrap_or_default(),
                                v.build
                                    .as_ref()
                                    .map(|v| format!("+{v}"))
                                    .unwrap_or_default()
                            )
                        })
                        .unwrap_or_default();
                    if resolved.fingerprint != descriptor.fingerprint
                        || version != descriptor.package_version
                    {
                        return Err(semantic_plugin::PluginError::new(
                            "interface_incompatible",
                            "Installed package fingerprint/version differs from plugin requirement",
                        ));
                    }
                    declarations.insert(descriptor.export.clone(), resolved.interface);
                    definitions.extend(resolved.definitions);
                }
                semantic_rpc::interface::ConformingImplementation::new(
                    implementation,
                    declarations,
                    definitions,
                )
                .map(|v| Arc::new(v) as Arc<dyn semantic_rpc::interface::InterfaceImplementation>)
                .map_err(|e| semantic_plugin::PluginError::new(e.code, e.message))
            });
        let result = Self {
            runtime,
            db,
            changes: tokio::sync::Mutex::new(()),
            catalog,
        };
        let mut installed = result.list_internal(true).await?;
        for manifest in defaults {
            if installed.iter().any(|a| a.id == manifest.id) {
                continue;
            }
            let activation = PluginActivation {
                id: manifest.id.clone(),
                revision: manifest.revision,
                provider: PluginProvider::Rust { key: manifest.id },
                enabled: true,
                generation: 1,
                configuration: Value::Null,
                configuration_schema: manifest.configuration_schema,
                source_bindings: manifest.source_bindings,
                priority: None,
                exports: manifest
                    .exports
                    .into_iter()
                    .map(semantic_plugin::portable_export)
                    .collect(),
            };
            result.persist(&activation).await?;
            installed.push(activation);
        }
        for activation in installed {
            let _ = result.runtime.activate(activation).await;
        }
        Ok(result)
    }
    pub async fn list(&self) -> Result<Vec<PluginActivation>, AppError> {
        self.list_internal(false).await
    }
    async fn list_internal(
        &self,
        include_removed: bool,
    ) -> Result<Vec<PluginActivation>, AppError> {
        let mut query = SelectQuery::new().with_collection(COLLECTION);
        query.field_format = FieldFormat::Qualified;
        let QueryResult::Select(rows) = self.db.query_data(query.into()).await? else {
            return Err(error("invalid plugin query result"));
        };
        rows.into_iter().filter(|row| include_removed || !matches!(row.get(DESCRIPTOR_ATTR),Some(Value::Object(descriptor)) if descriptor.get("uninstalled") == Some(&Value::Bool(true)))).map(|row| PluginActivation::from_value(row.get(DESCRIPTOR_ATTR).ok_or_else(|| error("missing plugin descriptor"))?).map_err(error)).collect()
    }
    async fn persist(&self, activation: &PluginActivation) -> Result<(), AppError> {
        let mut object = Object::new();
        object.insert(ATTR_ID, activation.id.clone());
        object.insert(ATTR_TYPE, CLASS_ID.to_string());
        object.insert(DESCRIPTOR_ATTR, activation.to_value());
        self.db
            .execute_batch(Batch {
                operations: vec![BatchOperation::Upsert {
                    collection: COLLECTION.into(),
                    id: activation.id.clone(),
                    object,
                }],
                ..Default::default()
            })
            .await?;
        Ok(())
    }
    pub async fn configure(self: &Arc<Self>, activation: PluginActivation) -> Result<(), AppError> {
        let plugins = self.clone();
        tokio::spawn(async move { plugins.configure_inner(activation).await })
            .await
            .map_err(error)?
    }
    async fn configure_inner(&self, mut activation: PluginActivation) -> Result<(), AppError> {
        let _guard = self.changes.lock().await;
        let catalog = self.db.catalog().await?;
        *self
            .catalog
            .write()
            .map_err(|_| error("catalog lock poisoned"))? = catalog;
        let old = self
            .list_internal(true)
            .await?
            .into_iter()
            .find(|a| a.id == activation.id);
        activation.generation = old
            .map(|a| {
                a.generation
                    .checked_add(1)
                    .ok_or_else(|| error("plugin generation exhausted"))
            })
            .transpose()?
            .unwrap_or(1);
        activation.validate().map_err(error)?;
        self.runtime
            .validate_configuration(&activation)
            .map_err(error)?;
        // Persist desired state first; activation errors remain inspectable and can
        // be explicitly retried without losing the user's configuration.
        self.persist(&activation).await?;
        self.runtime.activate(activation).await.map_err(error)
    }
    pub async fn uninstall(self: &Arc<Self>, id: &str) -> Result<(), AppError> {
        let plugins = self.clone();
        let id = id.to_string();
        tokio::spawn(async move { plugins.uninstall_inner(&id).await })
            .await
            .map_err(error)?
    }
    async fn uninstall_inner(&self, id: &str) -> Result<(), AppError> {
        let _guard = self.changes.lock().await;
        if let Some(mut activation) = self.list().await?.into_iter().find(|a| a.id == id) {
            activation.enabled = false;
            activation.generation = activation
                .generation
                .checked_add(1)
                .ok_or_else(|| error("plugin generation exhausted"))?;
            self.runtime
                .activate(activation.clone())
                .await
                .map_err(error)?;
            // Preserve removal of code-registered defaults across scope reopen.
            // This is installation configuration, never import working state.
            let Value::Object(mut descriptor) = activation.to_value() else {
                unreachable!()
            };
            descriptor.insert("uninstalled", true);
            let mut object = Object::new();
            object.insert(ATTR_ID, id.to_string());
            object.insert(ATTR_TYPE, CLASS_ID.to_string());
            object.insert(DESCRIPTOR_ATTR, Value::Object(descriptor));
            self.db
                .execute_batch(Batch {
                    operations: vec![BatchOperation::Upsert {
                        collection: COLLECTION.into(),
                        id: id.into(),
                        object,
                    }],
                    ..Default::default()
                })
                .await?;
        }
        Ok(())
    }
}
