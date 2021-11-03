use std::{collections::HashMap, sync::Arc};

use anyhow::{anyhow, bail};
use factordb::{query::migrate::Migration, AnyError, Db};
use semantic_core::plugin::{DynPlugin, ImportOutput, Plugin, PluginSchema};
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

        let host = deno::DenoPluginHost::start(config).await?;

        state.deno = Some(host);
        Ok(())
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
                    bail!("Invalid migration '{}' (index {}): Migration was already applied, but has changed", name, index);
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

    pub async fn fetch_url(&self, url: url::Url) -> Result<Option<ImportOutput>, AnyError> {
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

        let plugin = if let Some(p) = plugin_opt {
            p
        } else {
            return Ok(None);
        };

        plugin.fetch_url(url).await
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

pub struct FetchResult {
    pub plugin_name: String,
    pub output: ImportOutput,
}
