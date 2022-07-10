use std::collections::{HashMap, HashSet};

use anyhow::bail;
use factordb::prelude::{
    AttrId, AttrMapExt, AttributeDescriptor, Batch, DataMap, Db, Expr, Id, Migration, Select,
};
use semantic_core::plugin::Plugin;

use crate::plugin::PluginManager;

pub mod logdb;

fn build_plugin_migration_name(
    plugin: &(dyn Plugin + Send + Sync + 'static),
    migration: &Migration,
) -> Result<String, anyhow::Error> {
    let flat_name = migration.name.as_ref().ok_or_else(|| {
        anyhow::anyhow!(
            "Plugin {} has an invalid migration: migrations must have a name",
            plugin.name(),
        )
    })?;
    // ATTENTION: do not change this calcuation!
    // Doing so would break all plugins with migrations and require a
    // database purge!
    Ok(format!("plugin/{}/{}", plugin.name(), flat_name))
}

pub async fn apply_plugin_migrations(
    db: &Db,
    plugin: &(dyn Plugin + Send + Sync + 'static),
) -> Result<(), anyhow::Error> {
    let existing_migrations = db.migrations().await?;

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
                        changes.push(format!("Changed Action: \n\nOLD: {:#?}\n\n{:#?}", old, new));
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

    Ok(())
}

pub async fn compact_db_history(
    db: &Db,
    log: &logfs::LogFs,
    plugins: &PluginManager,
) -> Result<(), anyhow::Error> {
    async fn try_copy_entities(db: &Db, db2: &Db, batch_size: usize) -> Result<(), anyhow::Error> {
        // TODO: lock database to prevent stale data!

        let select_limit = 5000;
        let mut persisted_ids = HashSet::<Id>::new();
        let mut pending_entities = HashMap::<Id, DataMap>::new();
        let mut last_id = Id::nil();

        let mut batch = Batch::new();

        let mut queries_complete = false;

        loop {
            if !queries_complete {
                let filter = Expr::gt(Expr::attr::<AttrId>(), last_id.clone());
                let select = Select::new().with_limit(select_limit).with_filter(filter);
                let items = db.select(select).await?.items;

                tracing::debug!(entities=%items.len(), "loaded entity page");

                if items.is_empty() {
                    queries_complete = true;
                    tracing::info!("all entities loaded, persisting remaining entities...");
                } else {
                    for item in items {
                        let data = item.data;
                        let id = data.get_id().unwrap();

                        let has_unmet_dependencies =
                            entity_data_related_ids(&data).any(|x| !persisted_ids.contains(&x));

                        if has_unmet_dependencies {
                            pending_entities.insert(id, data);
                        } else {
                            batch = batch.and_create(factordb::query::mutate::Create { id, data });
                            persisted_ids.insert(id);
                        }
                        last_id = id;

                        if batch.actions.len() >= batch_size {
                            let entity_count = batch.actions.len();
                            db2.batch(batch).await?;
                            tracing::debug!(%entity_count, "persisted batch");
                            batch = Batch::new();
                        }
                    }
                }
            }

            let pending_to_persist = pending_entities
                .iter()
                .filter_map(|(id, data)| {
                    let has_unmet_dependencies =
                        entity_data_related_ids(&data).any(|x| !persisted_ids.contains(&x));
                    if has_unmet_dependencies {
                        None
                    } else {
                        Some(id.clone())
                    }
                })
                .collect::<Vec<_>>();

            for id in &pending_to_persist {
                let data = pending_entities.remove(id).unwrap();
                batch = batch.and_create(factordb::query::mutate::Create {
                    id: id.clone(),
                    data,
                });
                last_id = id.clone();
            }

            if batch.actions.len() >= batch_size {
                let entity_count = batch.actions.len();
                db2.batch(batch).await?;
                tracing::debug!(%entity_count, "persisted batch");
                batch = Batch::new();
            }

            if queries_complete {
                if pending_entities.is_empty() {
                    break;
                } else if pending_to_persist.is_empty() {
                    bail!("Deadlock while trying to resolve entity reference dependencies!");
                }
            }
        }

        if !batch.actions.is_empty() {
            let entity_count = batch.actions.len();
            db2.batch(batch).await?;
            tracing::debug!(%entity_count, "persisted final batch");
        }

        Ok(())
    }

    tracing::info!("starting DB history compaction...");
    let new_prefix = "_x/";
    let batch_size = 10_000;

    let db2 = crate::db::logdb::LogDbStore::new_with_prefix(log.clone(), new_prefix.to_string())
        .build_db()
        .await
        .map_err(|err| {
            tracing::error!(?err, "Could not open logfs");
            err
        })?;

    tracing::info!("applying plugin migrations to new database...");
    for plugin in plugins.plugins_ordered().await {
        apply_plugin_migrations(&db2, &*plugin).await?;
    }

    tracing::info!("copying entities...");

    if let Err(error) = try_copy_entities(db, &db2, batch_size).await {
        tracing::error!(?error, "Entity copying failed - reverting");
        log.remove_prefix(new_prefix)?;
        return Err(error.context("Entity copying failed"));
    }

    let old_keys = log.paths_prefix(logdb::DEFAULT_PREFIX)?;
    let new_keys = log.paths_prefix(new_prefix)?;

    let renames = new_keys
        .into_iter()
        .map(|key| logfs::Rename {
            new_key: key.replacen(new_prefix, logdb::DEFAULT_PREFIX, 1),
            old_key: key,
        })
        .collect();

    tracing::info!("re-mapping keys...");

    let batch = logfs::Batch {
        deleted_keys: old_keys,
        renames,
    };

    log.batch(batch)?;

    tracing::info!("compaction complete!");

    Ok(())
}

fn entity_data_related_ids(data: &DataMap) -> impl Iterator<Item = Id> + '_ {
    data.iter().filter_map(|(key, value)| match (key, value) {
        (key, factordb::prelude::Value::Id(id)) if key != AttrId::QUALIFIED_NAME => {
            Some(id.clone())
        }
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use factordb::prelude::EntityContainer;
    use semantic_core::{
        api::{BackendConfig, BackendCryptoConfig},
        base::Note,
    };

    use crate::app::{App, AppConfig};

    use super::*;

    #[test]
    fn test_compact_logdb_history() {
        tracing_subscriber::fmt::init();

        let rt = tokio::runtime::Runtime::new().unwrap();

        let data_dir = std::env::temp_dir()
            .join("semantic")
            .join("test_compact_logdb_history");
        if data_dir.is_dir() {
            std::fs::remove_dir_all(&data_dir).unwrap();
        }

        let token_key = App::random_token_key();

        let app_config = AppConfig {
            backend: Some(BackendConfig {
                db: semantic_core::api::DbConfig::Crypto(BackendCryptoConfig {
                    data_path: Some(data_dir.join("db").to_str().unwrap().to_string()),
                    key: "key".to_string(),
                    key_iterations: Some(1),
                    salt: None,
                    raw: false,
                    offset: None,
                    full_index_write_interval: None,
                }),
                idle_timeout: None,
            }),
            token_key,
            deno: Some(crate::app::DenoConfig {
                data_dir: data_dir.join("deno"),
                plugin_dir: None,
            }),
            tmp_dir: None,
        };

        let handle = rt.handle().clone();

        rt.block_on(async move {
            let app = App::build(app_config.clone(), handle.clone())
                .await
                .unwrap();
            let db = app.db().unwrap();

            let note1 = Note {
                id: Id::random(),
                title: "note 1".to_string(),
                body: "note 1".to_string(),
                format: semantic_core::base::TextFormat::Plain,
                extra: Default::default(),
            };
            db.create_entity(note1.clone()).await.unwrap();

            let log = db
                .client()
                .as_any()
                .downcast_ref::<factor_engine::Engine>()
                .unwrap()
                .backend()
                .as_any()
                .unwrap()
                .downcast_ref::<factor_engine::backend::log::LogDb>()
                .unwrap()
                .with_store(|s| {
                    s.as_any()
                        .downcast_ref::<crate::db::logdb::LogDbStore>()
                        .unwrap()
                        .clone()
                })
                .await;

            let plugins = app.plugins().unwrap();

            compact_db_history(&db, log.log(), &plugins).await.unwrap();

            app.close_backend().await.unwrap();

            let app = App::build(app_config, handle.clone()).await.unwrap();
            let db = app.require_db().unwrap();

            let note1_raw = db.entity(note1.id).await.unwrap();
            let note1_a = Note::try_from_map(note1_raw).unwrap();
            assert_eq!(note1_a.title, "note 1");
        });
    }
}
