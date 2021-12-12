use std::{collections::HashMap, sync::Arc};

use anyhow::{anyhow, bail, Context};
use factordb::{
    data::value::patch::Patch,
    query::{migrate::Migration, mutate::Mutate},
    schema::{AttrMapExt, AttributeDescriptor, EntityDescriptor},
    AnyError, Db, Id,
};
use semantic_core::{
    api::PluginTestFetch,
    base::AttrComment,
    core::{AttrPluginCode, PluginSource},
    plugin::{
        DynPlugin, FetchUrlJob, FetchUrlOutput, ImportJob, ImportOutput, Plugin, PluginSchema,
    },
};
use tokio::sync::RwLock;

use super::deno;

#[derive(Clone)]
pub struct PluginManager(Arc<State>);

impl PluginManager {
    pub fn new(db: Db) -> Self {
        Self(Arc::new(State {
            db,
            mutable: RwLock::new(MutableState {
                plugins: HashMap::new(),
                deno: None,
            }),
        }))
    }

    pub async fn initialize_deno(&self, config: deno::DenoConfig) -> Result<(), AnyError> {
        let mut state = self.0.mutable.write().await;

        if state.deno.is_some() {
            bail!("Deno is already initialized");
        }

        let schema = self.0.db.schema()?;

        let host = deno::DenoPluginHost::start(config, schema).await?;

        state.deno = Some(host);
        Ok(())
    }

    pub async fn load_db_plugins(&self) -> Result<(), AnyError> {
        let page = self
            .0
            .db
            .select(PluginSource::query_all().with_limit(1000))
            .await?
            .convert_data::<PluginSource>()?;

        if page.next_cursor.is_some() {
            todo!("Handle additional pages");
        }

        for source in page.items {
            tracing::trace!(plugin=%source.ident, "initializing database plugin");
            match self.build_source_plugin(&source).await {
                Ok(plugin) => {
                    if let Err(err) = self.register_plugin(plugin).await {
                        tracing::error!(plugin=%source.ident, error=?err, "Could not restore database plugin");
                    }
                }
                Err(error) => {
                    tracing::error!(plugin=%source.ident, ?error, "Could not build plugin");
                }
            }
        }

        Ok(())
    }

    pub async fn plugin_source_validate(&self, source: PluginSource) -> Result<(), AnyError> {
        match source.runtime.as_ref().map(|x| x.as_str()) {
            Some("deno") => {}
            _ => {
                bail!("Unknown/missing plugin runtime")
            }
        }

        Ok(())
    }

    pub async fn create_source(&self, source: PluginSource) -> Result<PluginSource, AnyError> {
        let plugin = self.build_source_plugin(&source).await?;
        let source = PluginSource {
            id: source.id.non_nil_or_randomize(),
            ..source
        };
        self.0.db.create_entity(source.clone()).await?;
        self.register_plugin(plugin).await?;

        Ok(source)
    }

    pub async fn plugin_source_replace(
        &self,
        id: Id,
        code: String,
        comment: Option<String>,
    ) -> Result<PluginSource, AnyError> {
        let data = self.0.db.entity(id).await?;
        let mut source: PluginSource = data.try_into_entity()?;

        let mut patch = Patch::new();

        let plugin = if source.code.as_ref() != Some(&code) {
            // Source code has changed.
            // Validate and replace the plugin.

            if let Some(old) = self.0.mutable.write().await.plugins.remove(&source.ident) {
                old.plugin.stop()?;
            }

            patch = patch.replace(AttrPluginCode::QUALIFIED_NAME, code.clone());

            source.code = Some(code);

            let plugin = self.build_source_plugin(&source).await?;
            Some(plugin)
        } else {
            None
        };

        if comment.as_ref() != source.comment.as_ref() {
            patch = patch.replace(AttrComment::QUALIFIED_NAME, comment.clone());
            source.comment = comment;
        }

        if !patch.0.is_empty() {
            self.0.db.patch(id, patch.clone()).await?;
        }

        if let Some(plugin) = plugin {
            self.register_plugin(plugin).await?;
        }
        Ok(source)
    }

    pub async fn delete_plugin(&self, name: String) -> Result<(), AnyError> {
        let mut state = self.0.mutable.write().await;

        if let Some(plugin) = state.plugins.remove(&name) {
            if let Err(err) = plugin.plugin.stop() {
                tracing::warn!(?err, plugin=%name, "Could not properly stop plugin");
            }
        }

        let source = self.0.db.entity(name).await?;
        let source_ty = source
            .get_type_name()
            .ok_or_else(|| anyhow!("Plugin not found"))?;
        if source_ty != PluginSource::QUALIFIED_NAME {
            bail!("Plugin not found");
        }
        let id = source.get_id().ok_or_else(|| anyhow!("Plugin not found"))?;
        self.0.db.batch(Mutate::delete(id).into()).await?;

        Ok(())
    }

    async fn build_source_plugin(&self, source: &PluginSource) -> Result<DynPlugin, AnyError> {
        match source.runtime.as_ref().map(|s| s.as_str()) {
            Some("deno") => {}
            Some(other) => {
                bail!("Unsupported plugin runtime: {}", other);
            }
            None => {
                bail!("Plugins must have a runtime");
            }
        }

        let code = source
            .code
            .clone()
            .ok_or_else(|| anyhow!("Plugin must have source code"))?;

        let deno = { self.0.mutable.read().await.deno.clone() }
            .ok_or_else(|| anyhow!("Deno runtime not available"))?;

        let plugin = deno
            .register_plugin(deno::PluginSource { path: None, code }, true)
            .await
            .context("Deno failed to initialize plugin")?;

        if plugin.name() != source.ident {
            bail!("PluginSource ident does not match the plugin name specified in the schema");
        }

        Ok(plugin)
    }

    pub async fn register_plugin(&self, plugin: DynPlugin) -> Result<(), AnyError> {
        // Lock the state to prevent race conditions.
        let mut state = self.0.mutable.write().await;

        let plugin_name = plugin.name().to_string();

        if state.plugins.contains_key(&plugin_name) {
            bail!(
                "Can't register plugin '{}': plugin with same name already exists",
                plugin_name,
            );
        }

        let db = &self.0.db;
        let existing_migrations = db.backend().migrations().await?;

        // TODO: validate whole plugin schema.

        // Run migrations.
        let migrations = plugin.migrations();

        let mut new_migrations = Vec::new();

        for (index, mut migration) in migrations.into_iter().enumerate() {
            let name = build_plugin_migration_name(&*plugin, &migration)?;
            migration.name = Some(name.clone());

            // TODO: validate migration
            // Ensure that it only changes schema/data that is managed by the
            // plugin itself.

            let old_mig = existing_migrations
                .iter()
                .find(|n| n.name == migration.name);
            if let Some(old_migration) = old_mig {
                if old_migration != &migration {
                    let mut changes = Vec::new();

                    tracing::error!(
                        ?old_migration,
                        ?migration,
                        "already applied migration has changed"
                    );

                    for (old, new) in old_migration.actions.iter().zip(migration.actions.iter()) {
                        if old != new {
                            changes
                                .push(format!("Changed Action: \n\nOLD: {:#?}\n\n{:#?}", old, new));
                        }
                    }

                    let changes_text = changes.join("\n\n");

                    bail!("Invalid migration '{}' (index {}): Migration was already applied, but has changed\n\nCHANGES:\n{}", name, index, changes_text);
                }

                if !new_migrations.is_empty() {
                    bail!("Invalid migration '{}': invalid ordering: old migration comes after missing migration", name);
                }
            } else {
                new_migrations.push((name, migration));
            }
        }

        for (name, migration) in new_migrations {
            tracing::trace!(name= ?name, "Running plugin migration");
            db.migrate(migration).await?;
            tracing::trace!(name= ?name, "Plugin migration applied");
        }

        state.plugins.insert(
            plugin.name().to_string(),
            PluginItem {
                schema: plugin.schema(),
                plugin,
            },
        );

        tracing::debug!(name=%plugin_name, "Registered plugin");

        Ok(())
    }

    pub async fn fetch_url(&self, job: FetchUrlJob) -> Result<FetchUrlOutput, AnyError> {
        let url = job.url.clone();
        tracing::trace!(%url, "finding plugin to fetch url");

        // Find the most suited plugin.
        let plugin_opt = {
            self.0
                .mutable
                .read()
                .await
                .plugins
                .values()
                .filter_map(|item| {
                    let m = item.schema.find_import_match(&url)?;
                    Some((item.plugin.clone(), m))
                })
                .max_by(|a, b| a.1.cmp(&b.1))
                .map(|x| x.0)
        };

        let plugin =
            plugin_opt.ok_or_else(|| anyhow!("No suitable importer found for url '{}'", url))?;
        tracing::trace!(plugin=%plugin.name(), %url, "fetching url with plugin");
        let output = plugin
            .fetch_url(job)
            .await?
            .ok_or_else(|| anyhow!("No suitable importer found for url '{}'", url))?;
        Ok(output)
    }

    pub async fn import(&self, job: ImportJob) -> Result<ImportOutput, AnyError> {
        let url = job.url.clone();
        tracing::trace!(%url, "finding plugin to fetch url");

        // Find the most suited plugin.
        let plugin_opt = {
            self.0
                .mutable
                .read()
                .await
                .plugins
                .values()
                .filter_map(|item| {
                    let m = item.schema.find_import_match(&url)?;
                    Some((item.plugin.clone(), m))
                })
                .max_by(|a, b| a.1.cmp(&b.1))
                .map(|x| x.0)
        };

        let plugin =
            plugin_opt.ok_or_else(|| anyhow!("No suitable importer found for url '{}'", url))?;
        tracing::trace!(plugin=%plugin.name(), %url, "fetching url with plugin");
        let output = plugin
            .import(job)
            .await?
            .ok_or_else(|| anyhow!("No suitable importer found for url '{}'", url))?;
        Ok(output)
    }

    pub async fn test_fetch(
        &self,
        spec: PluginTestFetch,
    ) -> Result<Option<FetchUrlOutput>, AnyError> {
        if spec.runtime != "deno" {
            bail!("Unsupported runtime '{}'", spec.runtime);
        }

        let deno = {
            self.0
                .mutable
                .read()
                .await
                .deno
                .clone()
                .ok_or_else(|| anyhow!("Deno runtime not available"))?
        };

        deno.test_fetch(&spec.code, spec.url, true).await
    }
}

struct State {
    db: Db,
    mutable: RwLock<MutableState>,
}

struct PluginItem {
    schema: PluginSchema,
    plugin: DynPlugin,
}

struct MutableState {
    plugins: HashMap<String, PluginItem>,
    deno: Option<deno::DenoPluginHost>,
}

fn build_plugin_migration_name(
    plugin: &dyn Plugin,
    migration: &Migration,
) -> Result<String, AnyError> {
    let flat_name = migration.name.as_ref().ok_or_else(|| {
        anyhow!(
            "Plugin {} has an invalid migration: migrations must have a name",
            plugin.name(),
        )
    })?;
    // ATTENTION: do not change this calcuation!
    // Doing so would break all plugins with migrations and require a
    // database purge!
    Ok(format!("plugin/{}/{}", plugin.name(), flat_name))
}
