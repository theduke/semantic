use std::collections::BTreeMap;

use futures::{StreamExt, stream};
use semantic_data::value::Value;

use super::SourceScan;
use super::planner::LeafKey;
use super::pushdown::LeafFragment;
use crate::{
    AsyncPhysicalDataSource, CoreError, Expr, FieldRef, PhysicalIndexScan, PhysicalTextSearch,
    SendableRecordBatchStream, SourceRef,
};

pub(crate) struct CompositeDataSource {
    pub fragments: BTreeMap<LeafKey, LeafFragment>,
}

impl CompositeDataSource {
    fn read(&self, source: SourceRef, predicate: Option<&Expr>) -> SendableRecordBatchStream {
        let Some(fragment) = self.fragments.get(&LeafKey::from(&source)) else {
            return error_stream(format!("federation: no fragment for source {source:?}"));
        };
        if let Some(predicate) = predicate {
            debug_assert_eq!(fragment.pushed_predicate.as_ref(), Some(predicate));
        }
        let residual = fragment.residual.clone();
        fragment
            .source
            .clone()
            .scan(SourceScan {
                collection: fragment.collection.clone(),
                request: fragment.request.clone(),
                plan: fragment.plan.clone(),
                bindings: BTreeMap::new(),
            })
            .map(move |batch| {
                batch.map(|mut rows| {
                    if let Some(predicate) = &residual {
                        rows.retain(|row| crate::evaluate_filter_expr(row.as_ref(), predicate));
                    }
                    rows
                })
            })
            .boxed()
    }
}

fn error_stream(message: String) -> SendableRecordBatchStream {
    stream::once(async move { Err(CoreError::new(message)) }).boxed()
}

fn unsupported_index() -> SendableRecordBatchStream {
    error_stream("federation: index access is not supported on federated sources".into())
}

impl AsyncPhysicalDataSource for CompositeDataSource {
    fn scan_stream(&self, source: SourceRef) -> SendableRecordBatchStream {
        self.read(source, None)
    }
    fn scan_filtered_stream(
        &self,
        source: SourceRef,
        predicate: Expr,
    ) -> SendableRecordBatchStream {
        self.read(source, Some(&predicate))
    }
    fn index_lookup_stream(
        &self,
        _: SourceRef,
        _: FieldRef,
        _: Value,
    ) -> SendableRecordBatchStream {
        unsupported_index()
    }
    fn index_lookup_filtered_stream(
        &self,
        _: SourceRef,
        _: FieldRef,
        _: Value,
        _: Option<Expr>,
    ) -> SendableRecordBatchStream {
        unsupported_index()
    }
    fn index_lookup_limited_stream(
        &self,
        _: SourceRef,
        _: FieldRef,
        _: Value,
        _: Option<Expr>,
        _: Option<usize>,
    ) -> SendableRecordBatchStream {
        unsupported_index()
    }
    fn index_lookup_many_stream(
        &self,
        _: SourceRef,
        _: FieldRef,
        _: Vec<Value>,
        _: Option<Expr>,
    ) -> SendableRecordBatchStream {
        unsupported_index()
    }
    fn index_range_stream(&self, _: PhysicalIndexScan) -> SendableRecordBatchStream {
        unsupported_index()
    }
    fn text_search_stream(&self, _: PhysicalTextSearch) -> SendableRecordBatchStream {
        unsupported_index()
    }
}
