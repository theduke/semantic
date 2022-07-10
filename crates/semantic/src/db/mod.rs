pub mod logdb;
fn build_plugin_migration_name(
    plugin: &dyn Plugin,
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

pub async fn apply_plugin_migrations(db: &Db, plugin: &dyn Plugin) -> Result<(), anyhow::Error> {
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

