//! Diagnostics for the existing full package reconciliation path.

use std::time::Instant;

use crate::catalog::Catalog;

/// Compare persisted state only after reconciliation. New migrations necessarily
/// change their recorded history, so avoid diagnostic snapshots unless enabled.
pub(super) fn compare_catalogs(
    before: &Catalog,
    after: &Catalog,
    package: &str,
    attempt: u32,
    migrations_executed: usize,
) -> (bool, &'static str) {
    let diagnostics = tracing::enabled!(tracing::Level::DEBUG);
    if migrations_executed > 0 && !diagnostics {
        return (true, "migration");
    }
    let started = Instant::now();
    let mut before = before.to_storage_snapshot();
    let after = after.to_storage_snapshot();
    let changed = before != after;
    tracing::debug!(
        operation = "package_registration", package, attempt,
        phase = "snapshot_comparison", elapsed = ?started.elapsed(), catalog_changed = changed,
    );
    if changed && diagnostics {
        tracing::debug!(
            operation = "package_registration",
            package,
            attempt,
            phase = "catalog_difference",
            attributes = before.attributes != after.attributes,
            type_defs = before.type_defs != after.type_defs,
            record_types = before.record_types != after.record_types,
            classes = before.classes != after.classes,
            collections = before.collections != after.collections,
            indexes = before.indexes != after.indexes,
            relationships = before.relationships != after.relationships,
            packages = before.packages != after.packages,
            migrations = before.applied_migrations != after.applied_migrations,
            next_field_id = before.next_field_id != after.next_field_id,
            auto_index_enabled = before.auto_index_enabled != after.auto_index_enabled,
        );
    }
    let path = if !changed {
        "unchanged_reconciled"
    } else if migrations_executed > 0 {
        "migration"
    } else if diagnostics {
        // A metadata update can also repair schema. Classify the actual delta,
        // and keep this extra comparison/allocation behind tracing enablement.
        before.packages.clone_from(&after.packages);
        if before == after {
            "metadata_update"
        } else {
            "repair"
        }
    } else {
        "unobserved"
    };
    (changed, path)
}
