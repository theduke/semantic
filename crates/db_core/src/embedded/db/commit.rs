//! The commit point of runtime writes and change feed publication.

use super::*;
use crate::batch_return::ChangeSet;
use crate::{ChangeSource, ChangedEntity, CommittedChange};

/// How a runtime write is committed and reported to the change feed.
pub(super) struct CommitIntent {
    source: ChangeSource,
    /// Catalog installed by the commit, replacing `expected_version`.
    catalog: Option<(u64, Arc<Catalog>)>,
}

impl CommitIntent {
    /// A data write under the current catalog.
    pub(super) fn data(source: ChangeSource) -> Self {
        Self {
            source,
            catalog: None,
        }
    }

    /// A write that installs `catalog` over catalog version
    /// `expected_version` once the storage commit succeeded.
    pub(super) fn with_catalog(
        source: ChangeSource,
        expected_version: u64,
        catalog: Arc<Catalog>,
    ) -> Self {
        Self {
            source,
            catalog: Some((expected_version, catalog)),
        }
    }
}

impl<S: EntityStorage> EmbeddedDb<S> {
    /// Commit `ops` of a runtime write; every data- or catalog-changing
    /// write after opening goes through here.
    ///
    /// The storage commit is conditional on `expected_revision` when the
    /// storage detects conflicts. Once it succeeded, the intent's catalog is
    /// installed and one change event carrying `changes` is published, all
    /// while the caller holds exclusive access, so events follow revision
    /// order. Conflicts and errors publish nothing.
    pub(super) fn commit_write(
        &mut self,
        ops: &[StorageWriteOp],
        expected_revision: Option<u64>,
        intent: CommitIntent,
        changes: ChangeSet,
    ) -> Result<StorageCommitOutcome, DbError> {
        let outcome = if self.storage.tx_capabilities().conflict_detection {
            self.storage
                .apply_batch_conditional(ops, expected_revision)?
        } else {
            self.storage.apply_batch(ops)?;
            StorageCommitOutcome::Committed {
                revision: self.storage.current_revision()?,
            }
        };
        let StorageCommitOutcome::Committed { revision } = outcome else {
            return Ok(outcome);
        };
        let catalog_changed = intent.catalog.is_some();
        if let Some((expected_version, catalog)) = intent.catalog {
            self.catalog
                .compare_and_swap_arc(expected_version, catalog)
                .map_err(|mismatch| {
                    DbError::TransactionConflict(format!(
                        "catalog version changed: expected {}, actual {}",
                        mismatch.expected, mismatch.actual
                    ))
                })?;
        }
        self.publish_changes(
            revision.unwrap_or_default(),
            intent.source,
            catalog_changed,
            changes,
        );
        Ok(outcome)
    }

    fn publish_changes(
        &self,
        revision: u64,
        source: ChangeSource,
        catalog_changed: bool,
        changes: ChangeSet,
    ) {
        if (changes.is_empty() && !catalog_changed) || self.change_feed.subscriber_count() == 0 {
            return;
        }
        let catalog = self.catalog.snapshot();
        let changes = changes
            .into_iter()
            .filter_map(|((collection, id), change)| {
                let internal = catalog
                    .catalog
                    .collection_by_name(&collection)
                    .is_some_and(|schema| schema.internal);
                ChangedEntity::new(collection, id, change.before, change.after)
                    .map(|change| (change, internal))
            });
        self.change_feed.publish(CommittedChange::new(
            revision,
            source,
            catalog.version,
            catalog_changed,
            changes,
        ));
    }
}
