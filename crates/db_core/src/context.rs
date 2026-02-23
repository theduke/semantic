use std::sync::Arc;

use crate::catalog::{Catalog, SharedCatalog};

#[derive(Debug, Clone)]
pub struct QueryContext {
    catalog: Arc<Catalog>,
}

impl QueryContext {
    pub fn new(catalog: Arc<Catalog>) -> Self {
        Self { catalog }
    }

    pub fn from_shared(shared: &SharedCatalog) -> Self {
        Self {
            catalog: shared.catalog_arc(),
        }
    }

    pub fn catalog(&self) -> &Catalog {
        self.catalog.as_ref()
    }

    pub fn catalog_arc(&self) -> Arc<Catalog> {
        self.catalog.clone()
    }
}

impl Default for QueryContext {
    fn default() -> Self {
        Self {
            catalog: Arc::new(Catalog::new()),
        }
    }
}
