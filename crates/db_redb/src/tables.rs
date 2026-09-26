//! Physical table layout of [`RedbKvEngine`](crate::RedbKvEngine).
//!
//! The engine exposes one flat, ordered key space (see
//! [`semantic_db_kv::keys`]) but stores it in one redb table per key space,
//! selected by the key's leading tag byte:
//!
//! | Table      | Keys                                                             |
//! |------------|------------------------------------------------------------------|
//! | `meta`     | `0x01` meta entries (layout version), plus the engine revision   |
//! | `entities` | `0x02` entity rows                                               |
//! | `indexes`  | `0x03` index entries and `0x04` index format markers              |
//! | `other`    | every other key: `0x00`, reserved tags `0x05..`, legacy textual  |
//! |            | keys (migrated away by the `EntityStore` layout migration)        |
//!
//! Index entries and markers share a table: markers are few and tiny, and
//! both are only touched together by index maintenance. Tables store the
//! full logical key (including the tag), so routing a point operation is a
//! lookup of the first byte and every table is ordered like the logical key
//! space.
//!
//! Range scans are split at the table boundaries: [`split_range`] maps a
//! logical range onto per-table sub-ranges in key order, and scanning them
//! in that order and concatenating the results yields the logical scan. The
//! scans issued by `EntityStore` never span tables, so they read exactly one.
//!
//! The revision counter lives in the `meta` table under the engine-private
//! key [`REVISION_KEY`], which lies outside the logical meta key range
//! (`0x01..0x02`), so logical reads and scans never observe it.
//!
//! Databases written before this layout keep everything, including the
//! revision under `__semantic/revision`, in a single `kv` table.
//! [`prepare_tables`] moves them into the per-space tables when the engine
//! is opened, before `EntityStore` runs its own layout migration.

use std::sync::OnceLock;

use redb::{ReadableTable, TableDefinition};
use semantic_db_core::DbError;
use semantic_db_kv::keys::{TAG_ENTITY, TAG_INDEX, TAG_INDEX_MARKER, TAG_META, TAG_STATS};

use crate::storage_err;

type KvTableDefinition = TableDefinition<'static, &'static [u8], &'static [u8]>;
pub(crate) type KvTable<'txn> = redb::Table<'txn, &'static [u8], &'static [u8]>;
pub(crate) type KvReadOnlyTable = redb::ReadOnlyTable<&'static [u8], &'static [u8]>;

/// A physical table of the engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RedbTable {
    Meta,
    Entities,
    Indexes,
    Other,
}

impl RedbTable {
    pub const ALL: [Self; 4] = [Self::Meta, Self::Entities, Self::Indexes, Self::Other];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Meta => "meta",
            Self::Entities => "entities",
            Self::Indexes => "indexes",
            Self::Other => "other",
        }
    }

    /// Table storing `key`.
    pub fn for_key(key: &[u8]) -> Self {
        match key.first().copied() {
            Some(TAG_META) => Self::Meta,
            Some(TAG_ENTITY) => Self::Entities,
            Some(TAG_INDEX | TAG_INDEX_MARKER) => Self::Indexes,
            _ => Self::Other,
        }
    }

    const fn definition(self) -> KvTableDefinition {
        TableDefinition::new(self.name())
    }

    const fn slot(self) -> usize {
        self as usize
    }
}

// `SEGMENTS` relies on the routed tags being contiguous.
const _: () = assert!(
    TAG_META + 1 == TAG_ENTITY
        && TAG_ENTITY + 1 == TAG_INDEX
        && TAG_INDEX + 1 == TAG_INDEX_MARKER
        && TAG_INDEX_MARKER + 1 == TAG_STATS
);

/// The logical key space as contiguous ranges `[lower]..[upper]` of leading
/// bytes (unbounded when `None`), in key order, with the table storing each.
const SEGMENTS: [(RedbTable, Option<u8>, Option<u8>); 5] = [
    (RedbTable::Other, None, Some(TAG_META)),
    (RedbTable::Meta, Some(TAG_META), Some(TAG_ENTITY)),
    (RedbTable::Entities, Some(TAG_ENTITY), Some(TAG_INDEX)),
    (RedbTable::Indexes, Some(TAG_INDEX), Some(TAG_STATS)),
    (RedbTable::Other, Some(TAG_STATS), None),
];

/// A key range `start..end` (`start..` when `end` is `None`) within one table.
pub(crate) type TableRange = (RedbTable, Vec<u8>, Option<Vec<u8>>);

/// Split the logical range `start..end` (`start..` when `end` is `None`)
/// into non-empty per-table ranges in key order.
pub(crate) fn split_range(start: &[u8], end: Option<&[u8]>) -> Vec<TableRange> {
    SEGMENTS
        .iter()
        .filter_map(|&(table, lower, upper)| {
            let start = match lower {
                Some(lower) if [lower].as_slice() > start => vec![lower],
                _ => start.to_vec(),
            };
            let end = match (end, upper) {
                (Some(end), Some(upper)) => Some(end.min([upper].as_slice()).to_vec()),
                (Some(end), None) => Some(end.to_vec()),
                (None, upper) => upper.map(|upper| vec![upper]),
            };
            match &end {
                Some(end) if *end <= start => None,
                _ => Some((table, start, end)),
            }
        })
        .collect()
}

/// Engine-private key of the revision counter in the `meta` table.
pub(crate) const REVISION_KEY: &[u8] = b"engine/revision";
const _: () = assert!(REVISION_KEY[0] != TAG_META);

/// Single table of the legacy engine layout.
const LEGACY_KV_TABLE: KvTableDefinition = TableDefinition::new("kv");
/// Revision key of the legacy engine layout.
const LEGACY_REVISION_KEY: &[u8] = b"__semantic/revision";

fn decode_revision(bytes: &[u8]) -> Result<u64, DbError> {
    let bytes: [u8; 8] = bytes
        .try_into()
        .map_err(|_| DbError::Storage("invalid revision payload in redb metadata".to_string()))?;
    Ok(u64::from_be_bytes(bytes))
}

fn stored_revision(
    table: &impl ReadableTable<&'static [u8], &'static [u8]>,
) -> Result<Option<u64>, DbError> {
    table
        .get(REVISION_KEY)
        .map_err(storage_err)?
        .map(|value| decode_revision(value.value()))
        .transpose()
}

/// Revision stored in the `meta` table; `0` for a new database.
pub(crate) fn read_revision(
    meta: &impl ReadableTable<&'static [u8], &'static [u8]>,
) -> Result<u64, DbError> {
    Ok(stored_revision(meta)?.unwrap_or(0))
}

pub(crate) fn write_revision(meta: &mut KvTable<'_>, revision: u64) -> Result<(), DbError> {
    meta.insert(REVISION_KEY, revision.to_be_bytes().as_slice())
        .map_err(storage_err)?;
    Ok(())
}

/// The engine tables opened in one write transaction.
pub(crate) struct WriteTables<'txn> {
    tables: [KvTable<'txn>; 4],
}

impl<'txn> WriteTables<'txn> {
    pub(crate) fn open(txn: &'txn redb::WriteTransaction) -> Result<Self, DbError> {
        let [meta, entities, indexes, other] =
            RedbTable::ALL.map(|table| txn.open_table(table.definition()));
        Ok(Self {
            tables: [
                meta.map_err(storage_err)?,
                entities.map_err(storage_err)?,
                indexes.map_err(storage_err)?,
                other.map_err(storage_err)?,
            ],
        })
    }

    pub(crate) fn table(&self, table: RedbTable) -> &KvTable<'txn> {
        &self.tables[table.slot()]
    }

    pub(crate) fn table_mut(&mut self, table: RedbTable) -> &mut KvTable<'txn> {
        &mut self.tables[table.slot()]
    }
}

/// The engine tables of one read transaction, opened on first use.
pub(crate) struct ReadTables {
    txn: redb::ReadTransaction,
    tables: [OnceLock<KvReadOnlyTable>; 4],
}

impl ReadTables {
    pub(crate) fn new(txn: redb::ReadTransaction) -> Self {
        Self {
            txn,
            tables: Default::default(),
        }
    }

    pub(crate) fn table(&self, table: RedbTable) -> Result<&KvReadOnlyTable, DbError> {
        let slot = &self.tables[table.slot()];
        if let Some(opened) = slot.get() {
            return Ok(opened);
        }
        let opened = self
            .txn
            .open_table(table.definition())
            .map_err(storage_err)?;
        Ok(slot.get_or_init(|| opened))
    }
}

/// Outcome of [`prepare_tables`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TableSetup {
    /// All tables exist; nothing was written.
    UpToDate,
    /// Missing tables were created.
    Created,
    /// The legacy `kv` table was split into the engine tables.
    MigratedLegacy {
        /// Rows moved, excluding the legacy revision entry.
        rows: u64,
    },
}

fn table_exists(
    txn: &redb::ReadTransaction,
    definition: KvTableDefinition,
) -> Result<bool, DbError> {
    match txn.open_table(definition) {
        Ok(_) => Ok(true),
        Err(redb::TableError::TableDoesNotExist(_)) => Ok(false),
        Err(err) => Err(storage_err(err)),
    }
}

/// Create missing engine tables and migrate the legacy `kv` table.
///
/// Runs in one write transaction, so an interrupted migration leaves the
/// legacy table untouched and is retried on the next open. The logical
/// content is unchanged, so the revision is carried over as is. Opening an
/// up-to-date database only reads.
pub(crate) fn prepare_tables(db: &redb::Database) -> Result<TableSetup, DbError> {
    let (missing, legacy) = {
        let txn = db.begin_read().map_err(storage_err)?;
        let mut missing = false;
        for table in RedbTable::ALL {
            missing |= !table_exists(&txn, table.definition())?;
        }
        (missing, table_exists(&txn, LEGACY_KV_TABLE)?)
    };
    if !missing && !legacy {
        return Ok(TableSetup::UpToDate);
    }

    let txn = db.begin_write().map_err(storage_err)?;
    let outcome = {
        let mut tables = WriteTables::open(&txn)?;
        if legacy {
            migrate_legacy_table(&txn, &mut tables)?
        } else {
            TableSetup::Created
        }
    };
    if legacy {
        txn.delete_table(LEGACY_KV_TABLE).map_err(storage_err)?;
    }
    txn.commit().map_err(storage_err)?;
    Ok(outcome)
}

fn migrate_legacy_table(
    txn: &redb::WriteTransaction,
    tables: &mut WriteTables<'_>,
) -> Result<TableSetup, DbError> {
    let legacy = txn.open_table(LEGACY_KV_TABLE).map_err(storage_err)?;
    let mut rows = 0;
    let mut legacy_revision = None;
    for item in legacy.iter().map_err(storage_err)? {
        let (key, value) = item.map_err(storage_err)?;
        let (key, value) = (key.value(), value.value());
        if key == LEGACY_REVISION_KEY {
            legacy_revision = Some(decode_revision(value)?);
            continue;
        }
        tables
            .table_mut(RedbTable::for_key(key))
            .insert(key, value)
            .map_err(storage_err)?;
        rows += 1;
    }
    if let Some(legacy_revision) = legacy_revision {
        // Never move the revision backwards should the new tables already
        // hold one.
        let meta = tables.table_mut(RedbTable::Meta);
        let revision = stored_revision(meta)?.map_or(legacy_revision, |r| r.max(legacy_revision));
        write_revision(meta, revision)?;
    }
    Ok(TableSetup::MigratedLegacy { rows })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tables_of(start: &[u8], end: Option<&[u8]>) -> Vec<TableRange> {
        split_range(start, end)
    }

    #[test]
    fn routes_keys_by_tag() {
        assert_eq!(RedbTable::for_key(&[TAG_META, 1]), RedbTable::Meta);
        assert_eq!(RedbTable::for_key(&[TAG_ENTITY]), RedbTable::Entities);
        assert_eq!(RedbTable::for_key(&[TAG_INDEX, 9]), RedbTable::Indexes);
        assert_eq!(RedbTable::for_key(&[TAG_INDEX_MARKER]), RedbTable::Indexes);
        assert_eq!(RedbTable::for_key(&[TAG_STATS]), RedbTable::Other);
        assert_eq!(RedbTable::for_key(&[0]), RedbTable::Other);
        assert_eq!(RedbTable::for_key(b"c/1/e/x"), RedbTable::Other);
        assert_eq!(RedbTable::for_key(b""), RedbTable::Other);
        assert_eq!(RedbTable::for_key(REVISION_KEY), RedbTable::Other);
    }

    #[test]
    fn splits_ranges_at_table_boundaries() {
        // A range within one key space touches one table.
        assert_eq!(
            tables_of(&[TAG_ENTITY, 1], Some(&[TAG_ENTITY, 2])),
            vec![(
                RedbTable::Entities,
                vec![TAG_ENTITY, 1],
                Some(vec![TAG_ENTITY, 2])
            )]
        );
        assert_eq!(
            tables_of(b"b", Some(b"c")),
            vec![(RedbTable::Other, b"b".to_vec(), Some(b"c".to_vec()))]
        );
        // The full key space visits every segment in key order.
        let all = tables_of(&[], None);
        assert_eq!(
            all.iter().map(|(table, _, _)| *table).collect::<Vec<_>>(),
            [
                RedbTable::Other,
                RedbTable::Meta,
                RedbTable::Entities,
                RedbTable::Indexes,
                RedbTable::Other
            ]
        );
        assert_eq!(all[0], (RedbTable::Other, vec![], Some(vec![TAG_META])));
        assert_eq!(all[4], (RedbTable::Other, vec![TAG_STATS], None));
        // Partial overlaps are clamped; empty ranges yield nothing.
        assert_eq!(
            tables_of(&[TAG_ENTITY, 5], Some(&[TAG_INDEX_MARKER, 1])),
            vec![
                (
                    RedbTable::Entities,
                    vec![TAG_ENTITY, 5],
                    Some(vec![TAG_INDEX])
                ),
                (
                    RedbTable::Indexes,
                    vec![TAG_INDEX],
                    Some(vec![TAG_INDEX_MARKER, 1])
                ),
            ]
        );
        assert!(tables_of(b"c", Some(b"b")).is_empty());
        assert!(tables_of(&[TAG_META], Some(&[TAG_META])).is_empty());
    }
}
