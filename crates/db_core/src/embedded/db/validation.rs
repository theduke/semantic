use super::*;

pub(super) const STATE: &str = "__semantic.validation";
pub(super) const ACTIVE: &str = "recursive:v1";

impl<S: EntityStorage> EmbeddedDb<S> {
    pub(super) fn validate_catalog_rows(
        &self,
        catalog: &Catalog,
        after: &crate::Dataset,
    ) -> Result<(), DbError> {
        crate::validation::validate_enforcement_support(catalog)?;
        for (_, collection) in catalog.collections().filter(|(_, c)| !c.internal) {
            let stored;
            let rows = if let Some(rows) = after.get(&collection.name) {
                rows
            } else {
                stored = self
                    .storage
                    .scan_collection(collection.lid)?
                    .into_iter()
                    .map(|row| (row.id, row.object))
                    .collect::<BTreeMap<_, _>>();
                &stored
            };
            for (id, object) in rows {
                crate::validate_stored_object(
                    catalog,
                    &(collection.name.clone(), id.clone()),
                    object,
                    |key| {
                        if let Some(rows) = after.get(&key.0) {
                            return Ok(rows.get(&key.1).cloned());
                        }
                        let collection = catalog.collection_by_name(&key.0).ok_or_else(|| {
                            DbError::UnknownCollectionByName {
                                name: key.0.clone(),
                            }
                        })?;
                        Ok(self
                            .storage
                            .get_entity(collection.lid, &key.1)?
                            .map(|row| row.object))
                    },
                )?;
            }
        }
        Ok(())
    }
    /// Whether recursive stored-value and endpoint enforcement has been activated.
    pub fn validation_enabled(&self) -> Result<bool, DbError> {
        let catalog = self.catalog();
        let Some(collection) = catalog.collection_by_name(STATE) else {
            return Ok(false);
        };
        Ok(self.storage.get_entity(collection.lid, ACTIVE)?.is_some())
    }

    /// Inspect legacy data without normalizing, repairing, or writing anything.
    /// Reports the first violation per row; unsupported constraints are explicit errors.
    pub fn validation_preflight(&self) -> Result<Vec<crate::ValidationViolation>, DbError> {
        let catalog = self.catalog.snapshot();
        let revision = self.storage.current_revision()?;
        crate::validation::validate_enforcement_support(&catalog.catalog)?;
        let mut violations = Vec::new();
        for (_, collection) in catalog.catalog.collections().filter(|(_, c)| !c.internal) {
            for row in self.storage.scan_collection(collection.lid)? {
                let key = (collection.name.clone(), row.id.clone());
                match crate::validate_stored_object(&catalog.catalog, &key, &row.object, |key| {
                    let collection =
                        catalog.catalog.collection_by_name(&key.0).ok_or_else(|| {
                            DbError::UnknownCollectionByName {
                                name: key.0.clone(),
                            }
                        })?;
                    Ok(self
                        .storage
                        .get_entity(collection.lid, &key.1)?
                        .map(|row| row.object))
                }) {
                    Ok(_) => {}
                    Err(DbError::Validation(error)) => {
                        violations.push(crate::ValidationViolation {
                            collection: key.0,
                            id: key.1,
                            error,
                        })
                    }
                    Err(error) => return Err(error),
                }
            }
        }
        if self.storage.current_revision()? != revision
            || self.catalog.snapshot().version != catalog.version
        {
            return Err(DbError::TransactionConflict(
                "database changed during validation preflight".into(),
            ));
        }
        Ok(violations)
    }

    /// Activate only against a clean, unchanged preflight snapshot. The marker and
    /// complete reverse-reference backfill commit together and survive reopening.
    pub fn activate_validation(&mut self) -> Result<(), DbError> {
        if self.catalog().collection_by_name(STATE).is_none() {
            self.create_collection(STATE, CollectionKind::Untyped)?;
            self.mark_collection_internal(STATE)?;
        }
        let revision = self.storage.current_revision()?;
        if let Some(violation) = self.validation_preflight()?.into_iter().next() {
            return Err(violation.error.into());
        }
        let catalog = self.catalog.snapshot();
        let collection = catalog.catalog.collection_by_name(STATE).unwrap();
        let mut ops = Vec::new();
        self.rebuild_reverse_references(&catalog.catalog, &BTreeMap::new(), &mut ops)?;
        ops.push(StorageWriteOp::PutEntity(StoredEntity {
            collection: collection.lid.0,
            kind: StoredEntityKind::Untyped,
            id: ACTIVE.into(),
            object: Object::new(),
        }));
        if self.catalog.snapshot().version != catalog.version {
            return Err(DbError::TransactionConflict(
                "catalog changed during validation activation".into(),
            ));
        }
        match self.storage.apply_batch_conditional(&ops, revision)? {
            StorageCommitOutcome::Committed { .. } => Ok(()),
            StorageCommitOutcome::Conflict { .. } => Err(DbError::TransactionConflict(
                "database changed during validation activation".into(),
            )),
        }
    }
}

#[cfg(test)]
mod tests;
