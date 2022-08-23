use std::{
    collections::{HashMap, HashSet, VecDeque},
    task::Poll,
};

use anyhow::bail;
use factdb::{
    query::mutate, AttrId, AttrMapExt, AttributeMeta, Batch, DataMap, Db, Expr, Id, Item, Page,
    Select, Value, ValueMap,
};
use futures::{future::BoxFuture, StreamExt};
use semantic_core::plugin::Plugin;

use crate::plugin::PluginManager;

pub mod logdb;

fn plugin_migration_name_prefix(plugin_name: &str) -> String {
    // ATTENTION: do not change this calcuation!
    // Doing so would break all plugins with migrations and require a database purge!
    format!("plugin/{}/", plugin_name)
}

fn plugin_migration_name(plugin_name: &str, migration_name: &str) -> String {
    // ATTENTION: do not change this calcuation!
    // Doing so would break all plugins with migrations and require a database purge!
    let prefix = plugin_migration_name_prefix(plugin_name);
    format!("{prefix}{migration_name}")
}

fn filter_allowed_value(value: Value, all_ids: &HashSet<Id>) -> Option<Value> {
    if let Some(id) = value.as_id() {
        if all_ids.contains(&id) {
            Some(value)
        } else {
            None
        }
    } else if let Some(list) = value.as_list() {
        let clean = list
            .to_vec()
            .into_iter()
            .filter_map(|x| filter_allowed_value(x, all_ids))
            .collect::<Vec<_>>();
        if clean.is_empty() {
            None
        } else {
            Some(Value::List(clean))
        }
    } else {
        Some(value)
    }
}

pin_project_lite::pin_project! {
    pub struct EntitiesOrderedStream {
        db: Db,
        all_ids: HashSet<Id>,
        select_limit: u64,
        handled_ids: HashSet<Id>,
        pending_entities: HashMap<Id, DataMap>,
        last_id: Id,
        queries_finished: bool,

        popqueue: VecDeque<(Id, DataMap)>,

        #[pin]
        next_page_future: Option<BoxFuture<'static, Result<Page<Item>, anyhow::Error>>>,
    }
}

impl EntitiesOrderedStream {
    pub async fn new(db: Db, select_limit: u64) -> Result<Self, anyhow::Error> {
        let mut all_ids = HashSet::new();

        let mut last_id = Id::nil();
        loop {
            let filter = Expr::gt(Expr::attr::<AttrId>(), last_id.clone());
            let select = Select::new()
                .with_limit(select_limit)
                .with_filter(filter)
                .with_sort(Expr::attr::<AttrId>(), factdb::Order::Asc);
            let items = db.select_map(select).await?;

            if items.is_empty() {
                break;
            }

            for item in items {
                let id = item.get_id().unwrap();
                last_id = id;
                all_ids.insert(id);
            }
        }

        Ok(Self {
            db,
            all_ids,
            select_limit,
            handled_ids: HashSet::new(),
            pending_entities: HashMap::new(),
            last_id: Id::nil(),
            queries_finished: false,
            popqueue: VecDeque::new(),
            next_page_future: None,
        })
    }
}

impl futures::stream::Stream for EntitiesOrderedStream {
    type Item = Result<(Id, DataMap), anyhow::Error>;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        let mut this = self.project();

        loop {
            if let Some(f) = this.next_page_future.as_mut().get_mut() {
                match f.as_mut().poll(cx) {
                    Poll::Pending => {
                        return Poll::Pending;
                    }
                    Poll::Ready(Err(err)) => {
                        return Poll::Ready(Some(Err(err)));
                    }
                    Poll::Ready(Ok(page)) => {
                        *this.next_page_future = None;

                        let items = page.items;
                        if items.is_empty() {
                            *this.queries_finished = true;
                            tracing::info!("all entities loaded, forwarding remaining entities...");
                        } else {
                            for item in items {
                                let raw_data = item.data;
                                let clean_data = raw_data
                                    .0
                                    .into_iter()
                                    .filter_map(|(key, value)| {
                                        let val = filter_allowed_value(value, &this.all_ids)?;
                                        Some((key, val))
                                    })
                                    .collect();
                                let data = ValueMap(clean_data);

                                let id = data.get_id().unwrap();
                                *this.last_id = id;

                                if this.handled_ids.contains(&id)
                                    || this.pending_entities.contains_key(&id)
                                {
                                    continue;
                                }

                                let has_unmet_dependencies = entity_data_related_ids(&data)
                                    .any(|x| !this.handled_ids.contains(&x));

                                if has_unmet_dependencies {
                                    this.pending_entities.insert(id, data);
                                } else {
                                    this.handled_ids.insert(id);
                                    this.popqueue.push_back((id, data));
                                }
                            }
                        }

                        // Clean up pending entities.

                        loop {
                            let ready_entity_ids = this
                                .pending_entities
                                .iter()
                                .filter_map(|(id, data)| {
                                    let missing_ids =
                                        entity_data_related_ids(&data).collect::<Vec<_>>();
                                    let has_unmet_dependencies =
                                        missing_ids.iter().any(|x| !this.handled_ids.contains(&x));
                                    if has_unmet_dependencies {
                                        if *this.queries_finished {
                                            eprintln!(
                                                "Missing ids for entity id {id}: {missing_ids:?}"
                                            );
                                        }
                                        None
                                    } else {
                                        Some(id.clone())
                                    }
                                })
                                .collect::<Vec<_>>();

                            if ready_entity_ids.is_empty() {
                                break;
                            }

                            for id in &ready_entity_ids {
                                let data = this.pending_entities.remove(id).unwrap();
                                this.handled_ids.insert(*id);
                                this.popqueue.push_back((*id, data));
                            }
                        }
                    }
                }
            }

            if let Some(next) = this.popqueue.pop_front() {
                return Poll::Ready(Some(Ok(next)));
            } else if !*this.queries_finished {
                let filter = Expr::gt(Expr::attr::<AttrId>(), this.last_id.clone());
                let select = Select::new()
                    .with_limit(*this.select_limit)
                    .with_filter(filter)
                    .with_sort(Expr::attr::<AttrId>(), factdb::Order::Asc);
                let db = this.db.clone();

                let fut = Box::pin(async move { db.select(select).await });
                *this.next_page_future = Some(fut);
                continue;
            } else if !this.pending_entities.is_empty() {
                return Poll::Ready(Some(Err(anyhow::anyhow!(
                    "Deadlock while resolving entity dependencies: {}",
                    serde_json::to_string_pretty(&this.pending_entities).unwrap(),
                ))));
            } else {
                return Poll::Ready(None);
            }
        }
    }
}

pub async fn apply_plugin_migrations(
    db: &Db,
    plugin: &(dyn Plugin + Send + Sync + 'static),
) -> Result<(), anyhow::Error> {
    let migration_name_prefix = plugin_migration_name_prefix(plugin.name());

    let existing_migrations = db
        .migrations()
        .await?
        .into_iter()
        .filter(|m| {
            m.name
                .clone()
                .unwrap_or_default()
                .starts_with(&migration_name_prefix)
        })
        .collect::<Vec<_>>();

    // TODO: validate whole plugin schema.

    let existing_migration_names = existing_migrations
        .iter()
        .filter_map(|m| m.name.clone())
        .collect::<HashSet<_>>();

    let mut migration_names = HashSet::<String>::new();

    // Run migrations.
    let migrations = plugin.migrations(&existing_migration_names);

    let mut new_migrations = Vec::new();

    for (index, mut migration) in migrations.into_iter().enumerate() {
        let plain_name = migration.name.clone().ok_or_else(|| {
            anyhow::anyhow!(
                "Invalid migration at index {index}: plugin migrations must have a name"
            )
        })?;
        let full_name = plugin_migration_name(plugin.name(), &plain_name);
        migration.name = Some(full_name.clone());

        // TODO: validate migration
        // Ensure that it only changes schema/data that is managed by the
        // plugin itself.

        let old = existing_migrations
            .iter()
            .enumerate()
            .find(|(_index, n)| n.name == migration.name);

        if let Some((old_index, old_migration)) = old {
            if old_index != index {
                bail!("Invalid migration order: migration {plain_name} was previosly at index {old_index}, but is not at {index}");
            }

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

                bail!("Invalid migration '{}' (index {}): Migration was already applied, but has changed\n\nCHANGES:\n{}", full_name, index, changes_text);
            }

            if !new_migrations.is_empty() {
                bail!("Invalid migration '{}': invalid ordering: old migration comes after missing migration", full_name);
            }
        } else {
            new_migrations.push((full_name.clone(), migration));
        }

        migration_names.insert(full_name.clone());
    }

    for (index, old_name) in existing_migration_names.iter().enumerate() {
        if !migration_names.contains(old_name) {
            bail!("Invalid migrations: already applied migraiton {old_name} at index {index} was removed!");
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
    select_window_size: u64,
    batch_size: u64,
) -> Result<(), anyhow::Error> {
    async fn try_copy_entities(
        db: &Db,
        db2: &Db,
        batch_size: usize,
        select_window_size: u64,
    ) -> Result<(), anyhow::Error> {
        // TODO: lock database to prevent stale data!

        let select_window_size = std::cmp::min(select_window_size, batch_size as u64);

        let mut batch = Batch::new();
        let mut stream = EntitiesOrderedStream::new(db.clone(), select_window_size).await?;

        while let Some(res) = stream.next().await {
            let (id, data) = res?;

            batch = batch.and_create(mutate::Create { id, data });
            if batch.actions.len() >= batch_size {
                let entity_count = batch.actions.len();
                db2.batch(batch).await?;
                tracing::debug!(%entity_count, "persisted batch");
                batch = Batch::new();
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

    // Delete keys with the new prefix, which might be left over from a previous failed run.
    tracing::trace!("deleting left-over temporary keys...");
    log.remove_prefix(new_prefix)?;

    tracing::trace!("opening database overlay...");

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

    if let Err(error) = try_copy_entities(db, &db2, batch_size as usize, select_window_size).await {
        tracing::error!(?error, "Entity copying failed - reverting");
        log.remove_prefix(new_prefix)?;
        return Err(error.context("Entity copying failed"));
    }

    let old_keys = log.paths_prefix(logdb::DEFAULT_PREFIX)?;
    let new_keys = log.paths_prefix(new_prefix)?;

    let renames: Vec<_> = new_keys
        .into_iter()
        .map(|key| logfs::Rename {
            new_key: key.replacen(new_prefix, logdb::DEFAULT_PREFIX, 1),
            old_key: key,
        })
        .collect();

    tracing::info!("re-mapping keys...");

    let old_key_count = old_keys.len();
    let new_key_count = renames.len();
    let batch = logfs::Batch {
        deleted_keys: old_keys,
        renames,
    };

    log.batch(batch)?;

    tracing::info!(%old_key_count, %new_key_count, "compaction complete!");

    Ok(())
}

fn entity_data_related_ids(data: &DataMap) -> impl Iterator<Item = Id> + '_ {
    data.iter()
        .map(|(key, value)| -> Vec<Id> {
            if let Some(id) = value.as_id() {
                if key != AttrId::QUALIFIED_NAME {
                    return vec![id];
                } else {
                    vec![]
                }
            } else if let Some(list) = value.as_list() {
                list.iter().filter_map(|x| x.as_id()).collect::<Vec<_>>()
            } else {
                vec![]
            }
        })
        .flatten()
}

#[cfg(test)]
mod tests {
    use factdb::ClassContainer;
    use semantic_core::{
        api::{BackendConfig, BackendCryptoConfig},
        base::Note,
    };

    use crate::app::{App, AppConfig};

    use super::*;

    #[test]
    fn test_compact_logdb_history() {
        tracing_subscriber::fmt::try_init().ok();

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
                    readonly: false,
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

            let mut notes = Vec::new();
            for index in 0..500 {
                let note = Note {
                    id: Id::random(),
                    title: format!("note {index}"),
                    body: format!("note {index} body"),
                    format: semantic_core::base::TextFormat::Plain,
                    extra: Default::default(),
                };
                db.create_entity(note.clone()).await.unwrap();
                notes.push(note);
            }

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

            compact_db_history(&db, log.log(), &plugins, 2, 2)
                .await
                .unwrap();

            app.close_backend().await.unwrap();

            let app = App::build(app_config, handle.clone()).await.unwrap();
            let db = app.require_db().unwrap();

            for old_note in &notes {
                let note_raw = db.entity(old_note.id).await.unwrap();
                let note = Note::try_from_map(note_raw).unwrap();
                assert_eq!(note.title, old_note.title);
            }
        });
    }
}
