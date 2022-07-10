use std::{collections::HashMap, sync::Arc};

use anyhow::{anyhow, bail, Context};
use factordb::{
    prelude::{AttrMapExt, Db, EntityContainer, EntityDescriptor, Mutate},
    AnyError,
};
use semantic_core::{
    api::PluginTestFetch,
    core::PluginSource,
    plugin::{DynPlugin, FetchUrlJob, FetchUrlOutput, ImportJob, ImportOutput, PluginSchema},
};
use tokio::sync::RwLock;

use super::deno;

#[derive(Clone)]
pub struct PluginManager(Arc<State>);

struct State {
    db: Db,
    mutable: RwLock<MutableState>,
}

struct PluginItem {
    index: usize,
    schema: PluginSchema,
    plugin: DynPlugin,
}

struct MutableState {
    plugins: HashMap<String, PluginItem>,
    deno: Option<deno::DenoPluginHost>,
}

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

    /// Get all plugins, ordered by the time they were registered.

    pub async fn initialize_deno(&self, config: deno::DenoConfig) -> Result<(), AnyError> {
        let mut state = self.0.mutable.write().await;

        if state.deno.is_some() {
            bail!("Deno is already initialized");
        }

        let schema = self.0.db.schema().await?;

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
        new_source: PluginSource,
    ) -> Result<PluginSource, AnyError> {
        let data = self.0.db.entity(new_source.id).await?;
        let old_source: PluginSource = data.try_into_entity()?;

        let mut new_plugin = None;
        if let Some(code) = new_source.code.as_ref() {
            if Some(code) != old_source.code.as_ref() {
                // Source code has changed.
                // Validate and replace the plugin.

                if let Some(old) = self
                    .0
                    .mutable
                    .write()
                    .await
                    .plugins
                    .remove(&new_source.ident)
                {
                    old.plugin.stop()?;
                }

                new_plugin = Some(self.build_source_plugin(&new_source).await?);
            }
        }

        if new_source != old_source {
            self.0
                .db
                .replace(new_source.id, new_source.clone().into_map()?)
                .await?;
        }

        if let Some(plugin) = new_plugin {
            self.register_plugin(plugin).await?;
        }
        Ok(new_source)
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
            .register_plugin(
                deno::PluginSource { path: None, code },
                source.strict_validation,
            )
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

        crate::db::apply_plugin_migrations(&self.0.db, &*plugin).await?;

        let index = state.plugins.len();

        state.plugins.insert(
            plugin.name().to_string(),
            PluginItem {
                index,
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
