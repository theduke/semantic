//! Durable evidence that registration is a fixed point of the full algorithm.
//!
//! Certificates are produced only while encoding a catalog write, in the same
//! transaction as that catalog. They cover the *entire* persistent catalog and
//! exact package input, not just the package's final schema. Certification runs
//! the ordinary validator and historical DDL replay, including intermediate ID
//! allocation and global projection validation, without executing data work.
//!
//! Bump VERSION whenever normalization, validation, replay, or catalog loading
//! semantics change. Introduce a forward core migration when upgrading the
//! certificate algorithm so existing databases get certified on a real catalog
//! write. Unknown/missing certificates always use ordinary registration.

use std::collections::BTreeMap;
use std::sync::{Arc, Weak};
use std::time::Instant;

use semantic_data::schema::{Migration, MigrationOperation, Package};
use sha2::{Digest, Sha256};

use crate::catalog::{Catalog, CatalogSnapshot, CatalogStorageSnapshot};
use crate::{
    DbError, apply_migration_ddl_batch, normalize_package_definition,
    validate_package_migrations_with_catalog,
};

const VERSION: u32 = 1;
pub(super) const FIELD: &str = "registration_proofs";
#[cfg(test)]
thread_local! {
    static REPLAY_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(super) fn take_replay_count() -> usize {
    REPLAY_COUNT.with(|count| count.replace(0))
}

#[derive(Debug, Clone, facet::Facet)]
pub(super) struct RegistrationProofs {
    version: u32,
    catalog_digest: String,
    reopened_catalog_digest: Option<String>,
    packages: BTreeMap<String, String>,
}

#[derive(Debug, Default)]
pub(super) struct BoundRegistrationProofs {
    catalog: Weak<Catalog>,
    catalog_version: u64,
    proofs: Option<RegistrationProofs>,
}

impl BoundRegistrationProofs {
    pub(super) fn new(snapshot: &CatalogSnapshot, proofs: Option<RegistrationProofs>) -> Self {
        Self {
            catalog: Arc::downgrade(&snapshot.catalog),
            catalog_version: snapshot.version,
            proofs,
        }
    }

    pub(super) fn permits(&self, snapshot: &CatalogSnapshot, package: &str, digest: &str) -> bool {
        self.catalog_version == snapshot.version
            && self.catalog.ptr_eq(&Arc::downgrade(&snapshot.catalog))
            && self.proofs.as_ref().is_some_and(|proofs| {
                proofs.version == VERSION
                    && proofs.packages.get(package).is_some_and(|d| d == digest)
            })
    }
}

pub(super) fn digest<T: facet::Facet<'static>>(value: &T) -> Result<String, DbError> {
    let encoded =
        facet_json::to_string(value).map_err(|error| DbError::Serialization(error.to_string()))?;
    Ok(format!("{:x}", Sha256::digest(encoded.as_bytes())))
}

pub(super) fn reconcile_migration(
    catalog: &mut Catalog,
    migration: &Migration,
) -> Result<(), DbError> {
    #[cfg(test)]
    REPLAY_COUNT.with(|count| count.set(count.get() + 1));
    apply_migration_ddl_batch(
        catalog,
        &migration.module,
        migration
            .operations
            .iter()
            .filter_map(|operation| match operation {
                MigrationOperation::Ddl(operation) => Some(operation),
                MigrationOperation::Insert { .. }
                | MigrationOperation::Update { .. }
                | MigrationOperation::Delete { .. } => None,
            }),
    )
    .map_err(|error| DbError::InvalidQuery(error.to_string()))
}

fn check_fixed_point(
    catalog: &Catalog,
    package: &Package,
    expected: &CatalogStorageSnapshot,
) -> Result<(), &'static str> {
    validate_package_migrations_with_catalog(package, catalog).map_err(|_| "validation_error")?;
    let mut replayed = catalog.clone();
    for migration in &package.migrations {
        let Some(applied) =
            catalog.applied_migration(&package.name, &migration.module, &migration.name)
        else {
            return Err("missing_migration");
        };
        // A Log-policy mismatch must continue logging and replaying the stored
        // definition on every call. Never certify it, even after metadata updates.
        if applied.migration != *migration {
            return Err("migration_mismatch");
        }
        reconcile_migration(&mut replayed, &applied.migration)
            .map_err(|_| "reconciliation_error")?;
    }
    replayed.upsert_package(package.clone());
    if replayed.to_storage_snapshot() != *expected {
        return Err("reconciliation_changes_catalog");
    }
    Ok(())
}

impl RegistrationProofs {
    /// Called only on the catalog mutation path. A failed proof is not a new
    /// validation error: registration retains its established repair/error path.
    pub(super) fn certify(catalog: &Catalog) -> Result<Self, DbError> {
        let started = Instant::now();
        let snapshot = catalog.to_storage_snapshot();
        let mut proofs = Self {
            version: VERSION,
            catalog_digest: digest(&snapshot)?,
            reopened_catalog_digest: None,
            packages: BTreeMap::new(),
        };
        // Prove both the live state and the state constructed by reopening it.
        // Loading alone is not validation (e.g. it accepts inheritance cycles).
        if let Ok(reopened) = Catalog::from_storage_snapshot(snapshot.clone()) {
            // The loader can canonicalize stored relationships. Certify each
            // representation against its own full-replay result, never assume
            // loading leaves even the persistent snapshot unchanged.
            let reopened_snapshot = reopened.to_storage_snapshot();
            proofs.reopened_catalog_digest = Some(digest(&reopened_snapshot)?);
            for (_, package) in catalog.packages() {
                let result = if normalize_package_definition(package)
                    .is_ok_and(|normalized| normalized == *package)
                {
                    check_fixed_point(catalog, package, &snapshot)
                        .and_then(|()| check_fixed_point(&reopened, package, &reopened_snapshot))
                } else {
                    Err("unnormalized_package")
                };
                match result {
                    Ok(()) => {
                        proofs
                            .packages
                            .insert(package.name.clone(), digest(package)?);
                    }
                    Err(reason) => {
                        tracing::debug!(
                            operation = "package_registration",
                            phase = "certification",
                            package = package.name,
                            fallback_reason = reason
                        );
                    }
                }
            }
        }
        tracing::debug!(operation = "package_registration", phase = "certification",
            elapsed = ?started.elapsed(), certified_packages = proofs.packages.len());
        Ok(proofs)
    }

    /// The caller has already loaded all catalog rows. Hash the reconstructed
    /// catalog without another storage scan; a changed/legacy encoding fails closed.
    pub(super) fn verify(self, catalog: &Catalog) -> Option<Self> {
        if self.version != VERSION {
            return None;
        }
        let digest = digest(&catalog.to_storage_snapshot()).ok()?;
        (digest == self.catalog_digest || self.reopened_catalog_digest.as_ref() == Some(&digest))
            .then_some(self)
    }
}
