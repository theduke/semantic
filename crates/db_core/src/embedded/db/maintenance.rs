//! Maintenance operations of the embedded database (see
//! [`crate::maintenance`]).
//!
//! Rebuilds commit through [`EmbeddedDb::commit_write`] with
//! [`crate::ChangeSource::Maintenance`] and no row changes. Since neither
//! rows nor the catalog change, the change feed publishes nothing for them.

use std::path::Path;
use std::time::Instant;

use super::commit::CommitIntent;
use super::compact::REFERENCES;
use super::incremental::{CONTRIBUTORS, COUNTS};
use super::*;
use crate::{
    BackupReport, CompactReport, DerivedData, ReindexReport, ReindexTarget, ReindexedIndex,
    RepairReport, RewriteReport, StorageIntegrityCheck, VerifyOptions, VerifyReport,
};

mod verify;

#[cfg(test)]
mod tests;

use verify::{RepairPlan, Verifier};

impl<S: EntityStorage> EmbeddedDb<S> {
    /// Rebuild the entries of the indexes selected by `target` from the
    /// stored rows.
    ///
    /// Each index is rebuilt in one write transaction (`ResetIndex` followed
    /// by one `IndexEntity` per row, streamed from a storage snapshot), so
    /// readers see either the old or the rebuilt index. Storages maintaining
    /// entry counters recount them from the rebuilt entries.
    /// [`ReindexTarget::All`] also rebuilds the reverse references and the
    /// relationship contributors, counts and edges (with the indexes of
    /// their internal collections) and recounts all maintained counters.
    pub fn reindex(&mut self, target: &ReindexTarget) -> Result<ReindexReport, DbError> {
        let started = Instant::now();
        let catalog = self.catalog();
        let indexes = reindex_targets(&catalog, target)?;
        let mut report = ReindexReport::default();
        let mut rebuilt_collections = BTreeSet::new();
        if *target == ReindexTarget::All {
            report.derived = self.rebuild_derived_data(&catalog, true, true)?;
            rebuilt_collections = derived_collections(&catalog);
        }
        for index in indexes {
            if !rebuilt_collections.contains(&index.collection) {
                report.indexes.push(self.reindex_index(&catalog, &index)?);
            }
        }
        if *target == ReindexTarget::All && self.storage.rebuild_storage_stats()? {
            report.derived.push(DerivedData::Stats);
        }
        report.duration = started.elapsed();
        Ok(report)
    }

    /// Check the consistency of the current committed state (see
    /// [`Self::verify_reader`]), including the physical integrity check when
    /// requested.
    pub fn verify(&mut self, options: &VerifyOptions) -> Result<VerifyReport, DbError> {
        Ok(self.verify_with_plan(options)?.0)
    }

    /// Run the physical storage integrity check for a verify.
    ///
    /// Needs exclusive access; storages that refuse to run it while read
    /// snapshots are open (redb) report it as skipped, and callers should
    /// retry once concurrent reads finished.
    pub fn check_storage_integrity_for_verify(&mut self) -> Result<StorageIntegrityCheck, DbError> {
        match self.storage.check_storage_integrity() {
            Ok(true) => Ok(StorageIntegrityCheck::Intact),
            Ok(false) => Ok(StorageIntegrityCheck::Repaired),
            Err(error) => match error.storage_kind() {
                Some(StorageErrorKind::Unsupported) => {
                    Ok(StorageIntegrityCheck::Skipped(error.to_string()))
                }
                Some(StorageErrorKind::InvalidState) => Ok(StorageIntegrityCheck::Skipped(
                    format!("{error}; retry once concurrent reads finished"),
                )),
                Some(StorageErrorKind::Corruption) => {
                    Ok(StorageIntegrityCheck::Corrupt(error.to_string()))
                }
                _ => Err(error),
            },
        }
    }

    /// Check the consistency of the state observed by `reader` without
    /// writing anything: index entries, reverse references, relationship
    /// contributors, counts and edges, maintained counters and payloads (see
    /// `verify.rs` for the algorithm). `integrity` is the outcome of
    /// [`Self::check_storage_integrity_for_verify`], when it ran.
    ///
    /// Rows are streamed from the reader's snapshot; the database lock is
    /// not needed.
    pub fn verify_reader(
        reader: &DbReader<'_>,
        options: &VerifyOptions,
        integrity: Option<StorageIntegrityCheck>,
    ) -> Result<VerifyReport, DbError> {
        Ok(Self::verify_reader_with_plan(reader, options, integrity)?.0)
    }

    fn verify_reader_with_plan(
        reader: &DbReader<'_>,
        options: &VerifyOptions,
        integrity: Option<StorageIntegrityCheck>,
    ) -> Result<(VerifyReport, RepairPlan), DbError> {
        let mut verifier =
            Verifier::<S>::new(reader.catalog(), reader.storage_snapshot(), *options);
        if let Some(integrity) = integrity {
            verifier.integrity(integrity);
        }
        verifier.run()
    }

    fn verify_with_plan(
        &mut self,
        options: &VerifyOptions,
    ) -> Result<(VerifyReport, RepairPlan), DbError> {
        let integrity = options
            .check_storage_integrity
            .then(|| self.check_storage_integrity_for_verify())
            .transpose()?;
        let reader = self.reader()?;
        Self::verify_reader_with_plan(&reader, options, integrity)
    }

    /// Verify with `options`, rebuild what the problems call for (indexes,
    /// reverse references, relationship data, counters) and verify again.
    ///
    /// Corrupt payloads and physical damage cannot be repaired and remain in
    /// the final report.
    pub fn repair(&mut self, options: &VerifyOptions) -> Result<RepairReport, DbError> {
        let (before, plan) = self.verify_with_plan(options)?;
        let started = Instant::now();
        let catalog = self.catalog();
        let mut rebuilt = ReindexReport::default();
        let mut rebuilt_collections = BTreeSet::new();
        if plan.reverse_references || plan.relationship_edges {
            rebuilt.derived = self.rebuild_derived_data(
                &catalog,
                plan.reverse_references,
                plan.relationship_edges,
            )?;
            rebuilt_collections = derived_collections(&catalog);
        }
        for lid in &plan.indexes {
            let Some(index) = catalog.index_by_lid(*lid) else {
                continue;
            };
            if !rebuilt_collections.contains(&index.collection) {
                rebuilt.indexes.push(self.reindex_index(&catalog, index)?);
            }
        }
        if plan.stats && self.storage.rebuild_storage_stats()? {
            rebuilt.derived.push(DerivedData::Stats);
        }
        rebuilt.duration = started.elapsed();
        let after = self.verify(options)?;
        Ok(RepairReport {
            before,
            rebuilt,
            after,
        })
    }

    /// Compact the physical storage (see
    /// [`EntityStorage::compact_storage`]).
    ///
    /// Storages that cannot compact while read snapshots are open (redb)
    /// fail with an `InvalidState` storage error; callers should retry once
    /// concurrent reads finished.
    pub fn compact_storage(&mut self) -> Result<CompactReport, DbError> {
        let started = Instant::now();
        let before = self.storage.storage_stats()?;
        let compacted = self.storage.compact_storage().map_err(|error| {
            if error.storage_kind() == Some(StorageErrorKind::InvalidState) {
                DbError::storage(
                    StorageErrorKind::InvalidState,
                    format!("{error}; retry the compaction once concurrent reads finished"),
                )
            } else {
                error
            }
        })?;
        let after = self.storage.storage_stats()?;
        let freed_bytes = before
            .file_size_bytes
            .zip(after.file_size_bytes)
            .map(|(before, after)| before.saturating_sub(after));
        Ok(CompactReport {
            before,
            after,
            compacted,
            freed_bytes,
            duration: started.elapsed(),
        })
    }

    /// Physical storage statistics (see [`EntityStorage::storage_stats`]).
    pub fn storage_stats(&self) -> Result<crate::embedded::StorageStats, DbError> {
        self.storage.storage_stats()
    }

    /// Rewrite every stored payload not in the storage's current format,
    /// `batch_size` rows per write transaction (see
    /// [`Self::rewrite_payload_batch`]).
    pub fn rewrite_payloads(&mut self, batch_size: usize) -> Result<RewriteReport, DbError> {
        let started = Instant::now();
        let mut report = RewriteReport::default();
        let mut resume_after = None;
        loop {
            let batch = self.rewrite_payload_batch(resume_after.as_deref(), batch_size)?;
            report.scanned += batch.scanned;
            report.rewritten += batch.rewritten;
            report.batches += u64::from(batch.rewritten > 0);
            resume_after = batch.resume_after;
            if resume_after.is_none() {
                break;
            }
        }
        report.duration = started.elapsed();
        Ok(report)
    }

    /// Rewrite one batch of payloads (see
    /// [`EntityStorage::rewrite_payload_batch`]). Entity contents do not
    /// change, so nothing is published to the change feed.
    pub fn rewrite_payload_batch(
        &mut self,
        resume_after: Option<&[u8]>,
        batch_size: usize,
    ) -> Result<crate::embedded::PayloadRewriteBatch, DbError> {
        if batch_size == 0 {
            return Err(DbError::InvalidQuery(
                "payload rewrite batch size must be positive".into(),
            ));
        }
        self.storage.rewrite_payload_batch(resume_after, batch_size)
    }

    /// Capture the current committed state for a backup (see
    /// [`EntityStorage::backup_source`]).
    pub fn backup_source(&self) -> Result<Box<dyn crate::embedded::BackupSource>, DbError> {
        self.storage.backup_source()
    }

    /// Write a consistent copy of the current committed state to a new
    /// database at `path`.
    pub fn backup(&self, path: &Path) -> Result<BackupReport, DbError> {
        write_backup(self.backup_source()?, path)
    }

    /// Rebuild the entries of `index` from the rows of its collection in one
    /// write transaction.
    fn reindex_index(
        &mut self,
        catalog: &Catalog,
        index: &crate::catalog::IndexSchema,
    ) -> Result<ReindexedIndex, DbError> {
        let collection = catalog
            .collection_by_lid(index.collection)
            .ok_or(DbError::UnknownCollection(index.collection))?;
        let revision = self.storage.current_revision()?;
        let mut ops = vec![StorageWriteOp::ResetIndex(index.lid)];
        let mut rows = 0;
        {
            let snapshot = self.storage.snapshot()?;
            for row in snapshot.scan_collection_stream(collection.lid)? {
                let row = row?;
                rows += 1;
                ops.push(StorageWriteOp::IndexEntity {
                    index: index.clone(),
                    entity_id: row.id,
                    object: row.object,
                });
            }
        }
        self.commit_maintenance(&ops, revision, "reindex")?;
        Ok(ReindexedIndex {
            collection: collection.name.clone(),
            index: index.schema.name.clone(),
            rows,
            entries: self.storage.index_entry_count(index.lid)?,
        })
    }

    /// Rebuild reverse references and/or relationship data (contributors,
    /// counts and edges, which also rebuilds the reverse references) from
    /// the rows in one write transaction.
    fn rebuild_derived_data(
        &mut self,
        catalog: &Catalog,
        reverse_references: bool,
        relationship_edges: bool,
    ) -> Result<Vec<DerivedData>, DbError> {
        let revision = self.storage.current_revision()?;
        let mut ops = Vec::new();
        let mut rebuilt = Vec::new();
        let no_rows = BTreeMap::new();
        if relationship_edges {
            self.rebuild_relationship_edges(catalog, &no_rows, &mut ops)?;
            rebuilt.extend([
                DerivedData::RelationshipEdges,
                DerivedData::ReverseReferences,
            ]);
        } else if reverse_references {
            self.rebuild_reverse_references(catalog, &no_rows, &mut ops)?;
            rebuilt.push(DerivedData::ReverseReferences);
        }
        self.commit_maintenance(&ops, revision, "rebuilding derived data")?;
        Ok(rebuilt)
    }

    fn commit_maintenance(
        &mut self,
        ops: &[StorageWriteOp],
        revision: Option<u64>,
        operation: &str,
    ) -> Result<(), DbError> {
        if ops.is_empty() {
            return Ok(());
        }
        let intent = CommitIntent::data(crate::ChangeSource::Maintenance);
        match self.commit_write(ops, revision, intent, Default::default())? {
            StorageCommitOutcome::Committed { .. } => Ok(()),
            StorageCommitOutcome::Conflict { .. } => Err(DbError::TransactionConflict(format!(
                "database changed during {operation}"
            ))),
        }
    }
}

/// Write the state captured by `source` to `path` and report it.
pub fn write_backup(
    source: Box<dyn crate::embedded::BackupSource>,
    path: &Path,
) -> Result<BackupReport, DbError> {
    let started = Instant::now();
    let backup = source.write_to(path)?;
    let bytes = std::fs::metadata(path).ok().map(|metadata| metadata.len());
    Ok(BackupReport {
        path: path.to_path_buf(),
        revision: backup.revision,
        entries: backup.entries,
        bytes,
        duration: started.elapsed(),
    })
}

/// The indexes selected by `target`.
fn reindex_targets(
    catalog: &Catalog,
    target: &ReindexTarget,
) -> Result<Vec<crate::catalog::IndexSchema>, DbError> {
    let collection_indexes = |name: &str| {
        let collection =
            catalog
                .collection_by_name(name)
                .ok_or_else(|| DbError::UnknownCollectionByName {
                    name: name.to_string(),
                })?;
        Ok::<_, DbError>(catalog.indexes_for_collection(collection.lid))
    };
    Ok(match target {
        ReindexTarget::All => catalog.indexes().map(|(_, index)| index.clone()).collect(),
        ReindexTarget::Collection(name) => collection_indexes(name)?.cloned().collect(),
        ReindexTarget::Index { collection, name } => {
            let index = collection_indexes(collection)?
                .find(|index| index.schema.name == *name)
                .ok_or_else(|| {
                    DbError::InvalidQuery(format!(
                        "collection '{collection}' has no index '{name}'"
                    ))
                })?;
            vec![index.clone()]
        }
    })
}

/// Internal collections rebuilt with the derived data, whose indexes are
/// rebuilt with them.
fn derived_collections(catalog: &Catalog) -> BTreeSet<LocalCollectionId> {
    [REFERENCES, CONTRIBUTORS, COUNTS, RELATION_EDGES_COLLECTION]
        .into_iter()
        .filter_map(|name| catalog.collection_by_name(name))
        .map(|collection| collection.lid)
        .collect()
}
