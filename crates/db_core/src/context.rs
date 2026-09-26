use std::sync::Arc;

use crate::MetricsCollector;
use crate::catalog::{Catalog, SharedCatalog};

#[derive(Debug, Clone)]
pub struct QueryContext {
    catalog: Arc<Catalog>,
    /// Sink of the execution's metrics, when collected.
    metrics: Option<Arc<MetricsCollector>>,
    /// Path of the operator being executed (child indexes from the root),
    /// maintained only while operators are profiled.
    operator_path: Option<Vec<u32>>,
}

impl QueryContext {
    pub fn new(catalog: Arc<Catalog>) -> Self {
        Self {
            catalog,
            metrics: None,
            operator_path: None,
        }
    }

    pub fn from_shared(shared: &SharedCatalog) -> Self {
        Self::new(shared.catalog_arc())
    }

    /// Collect execution metrics into `metrics` (and per-operator statistics
    /// when it profiles operators).
    pub fn with_metrics(mut self, metrics: Arc<MetricsCollector>) -> Self {
        self.operator_path = metrics.profiles_operators().then(Vec::new);
        self.metrics = Some(metrics);
        self
    }

    pub fn catalog(&self) -> &Catalog {
        self.catalog.as_ref()
    }

    pub fn catalog_arc(&self) -> Arc<Catalog> {
        self.catalog.clone()
    }

    /// The metrics sink of the execution, if any.
    pub fn metrics(&self) -> Option<&Arc<MetricsCollector>> {
        self.metrics.as_ref()
    }

    /// Path of the operator being executed, when operators are profiled.
    pub(crate) fn operator_path(&self) -> Option<&[u32]> {
        self.operator_path.as_deref()
    }

    /// The context of input `index` of the current operator.
    pub(crate) fn child(&self, index: u32) -> Self {
        let mut child = self.clone();
        if let Some(path) = &mut child.operator_path {
            path.push(index);
        }
        child
    }

    /// The context of work outside the profiled plan (expression
    /// subqueries planned at run time): metrics are still collected, but no
    /// operators are recorded.
    pub(crate) fn detached(&self) -> Self {
        Self {
            operator_path: None,
            ..self.clone()
        }
    }
}

impl Default for QueryContext {
    fn default() -> Self {
        Self::new(Arc::new(Catalog::new()))
    }
}
