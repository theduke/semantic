use std::sync::{Arc, RwLock};

use crate::catalog::Catalog;

#[derive(Debug, Clone)]
pub struct CatalogSnapshot {
    pub version: u64,
    pub catalog: Arc<Catalog>,
}

#[derive(Debug, Clone)]
pub struct SharedCatalog {
    inner: Arc<RwLock<CatalogState>>,
}

#[derive(Debug)]
struct CatalogState {
    version: u64,
    catalog: Arc<Catalog>,
}

impl SharedCatalog {
    pub fn new(catalog: Catalog) -> Self {
        Self {
            inner: Arc::new(RwLock::new(CatalogState {
                version: 0,
                catalog: Arc::new(catalog),
            })),
        }
    }

    pub fn empty() -> Self {
        Self::new(Catalog::new())
    }

    pub fn snapshot(&self) -> CatalogSnapshot {
        let guard = self.inner.read().expect("catalog lock poisoned");
        CatalogSnapshot {
            version: guard.version,
            catalog: guard.catalog.clone(),
        }
    }

    pub fn catalog_arc(&self) -> Arc<Catalog> {
        self.snapshot().catalog
    }

    pub fn replace(&self, catalog: Catalog) -> u64 {
        let mut guard = self.inner.write().expect("catalog lock poisoned");
        guard.version = guard.version.saturating_add(1);
        guard.catalog = Arc::new(catalog);
        guard.version
    }

    pub fn compare_and_swap(
        &self,
        expected_version: u64,
        catalog: Catalog,
    ) -> Result<u64, CatalogVersionMismatch> {
        let mut guard = self.inner.write().expect("catalog lock poisoned");
        if guard.version != expected_version {
            return Err(CatalogVersionMismatch {
                expected: expected_version,
                actual: guard.version,
            });
        }
        guard.version = guard.version.saturating_add(1);
        guard.catalog = Arc::new(catalog);
        Ok(guard.version)
    }
}

impl Default for SharedCatalog {
    fn default() -> Self {
        Self::empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("catalog version mismatch: expected {expected}, actual {actual}")]
pub struct CatalogVersionMismatch {
    pub expected: u64,
    pub actual: u64,
}
