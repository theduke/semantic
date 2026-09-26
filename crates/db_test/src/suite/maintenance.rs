use super::*;
use futures::StreamExt as _;
use semantic_db_core::{ReindexTarget, StorageErrorKind, VerifyOptions};

fn unsupported(error: &DbError) -> bool {
    error.storage_kind() == Some(StorageErrorKind::Unsupported)
}

/// Run after the other suites: every write they made must have left the
/// indexes, derived data and counters consistent with the rows. Backends
/// without maintenance operations are skipped.
pub async fn test_maintenance(db: &Db) {
    let options = VerifyOptions::all();
    let report = match db.verify(options).await {
        Ok(report) => report,
        Err(error) if unsupported(&error) => return,
        Err(error) => panic!("verify failed: {error}"),
    };
    assert!(report.is_ok(), "{report}");
    assert!(report.checked.rows > 0);

    let export = db.export_snapshot().await.unwrap();
    assert_eq!(export.revision, report.revision);
    let exported = export
        .stream
        .map(|row| row.unwrap())
        .collect::<Vec<_>>()
        .await;
    let scanned = db
        .scan_entities()
        .await
        .unwrap()
        .map(|row| row.unwrap())
        .collect::<Vec<_>>()
        .await;
    assert_eq!(exported.len(), scanned.len());
    assert!(
        exported
            .iter()
            .all(|row| export.catalog.collection_by_name(&row.collection).is_some())
    );

    let reindex = db.reindex(ReindexTarget::All).await.unwrap();
    assert!(!reindex.indexes.is_empty());
    let after = db.verify(options).await.unwrap();
    assert!(after.is_ok(), "{after}");
    assert_eq!(after.checked.rows, report.checked.rows);
    assert_eq!(after.checked.index_entries, report.checked.index_entries);

    let repair = db.repair(options).await.unwrap();
    assert!(repair.before.is_ok() && repair.after.is_ok(), "{repair}");
    assert!(repair.rebuilt.indexes.is_empty());

    match db.rewrite_payloads(64).await {
        Ok(rewrite) => assert_eq!(rewrite.rewritten, 0, "{rewrite}"),
        Err(error) => assert!(unsupported(&error), "{error}"),
    }
    if let Err(error) = db.storage_stats().await {
        assert!(unsupported(&error), "{error}");
    }
}
