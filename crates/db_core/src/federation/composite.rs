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
    fn read(
        &self,
        source: SourceRef,
        predicate: Option<&Expr>,
        lookup: Option<(FieldRef, Vec<Value>)>,
    ) -> SendableRecordBatchStream {
        let Some(fragment) = self.fragments.get(&LeafKey::from(&source)) else {
            return error_stream(format!("federation: no fragment for source {source:?}"));
        };
        if let Some(predicate) = predicate {
            debug_assert_eq!(fragment.pushed_predicate.as_ref(), Some(predicate));
        }
        let bindings = match (&fragment.bind_field, lookup) {
            (Some(expected), Some((field, values))) if expected == &field => {
                BTreeMap::from([(super::bind::KEYS_PARAMETER.into(), Value::List(values))])
            }
            (None, None) => BTreeMap::new(),
            _ => return error_stream("federation: scan does not match negotiated lookup".into()),
        };
        let residual = match fragment
            .residual
            .as_ref()
            .map(|predicate| bind_residual(predicate, &bindings))
            .transpose()
        {
            Ok(residual) => residual,
            Err(error) => return error_stream(error.to_string()),
        };
        fragment
            .source
            .clone()
            .scan(SourceScan {
                collection: fragment.collection.clone(),
                request: fragment.request.clone(),
                plan: fragment.plan.clone(),
                bindings,
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

pub(super) fn bind_residual(
    predicate: &Expr,
    bindings: &BTreeMap<String, Value>,
) -> Result<Expr, CoreError> {
    let mut predicate: semantic_data::query::Expr = predicate.clone().into();
    predicate.visit_mut(&mut |expr| {
        if let semantic_data::query::Expr::Operand(semantic_data::query::Operand::Parameter(name)) =
            expr
        {
            let value = bindings.get(name).ok_or_else(|| {
                CoreError::new(format!("federation: missing residual parameter '{name}'"))
            })?;
            *expr = semantic_data::query::Expr::Operand(semantic_data::query::Operand::Literal(
                value.clone(),
            ));
        }
        Ok(())
    })?;
    Ok(predicate.into())
}

fn error_stream(message: String) -> SendableRecordBatchStream {
    stream::once(async move { Err(CoreError::new(message)) }).boxed()
}

fn unsupported_index() -> SendableRecordBatchStream {
    error_stream("federation: index access is not supported on federated sources".into())
}

impl AsyncPhysicalDataSource for CompositeDataSource {
    fn scan_stream(&self, source: SourceRef) -> SendableRecordBatchStream {
        self.read(source, None, None)
    }
    fn scan_filtered_stream(
        &self,
        source: SourceRef,
        predicate: Expr,
    ) -> SendableRecordBatchStream {
        self.read(source, Some(&predicate), None)
    }
    fn index_lookup_stream(
        &self,
        source: SourceRef,
        field: FieldRef,
        value: Value,
    ) -> SendableRecordBatchStream {
        self.read(source, None, Some((field, vec![value])))
    }
    fn index_lookup_filtered_stream(
        &self,
        source: SourceRef,
        field: FieldRef,
        value: Value,
        residual_predicate: Option<Expr>,
    ) -> SendableRecordBatchStream {
        self.read(
            source,
            residual_predicate.as_ref(),
            Some((field, vec![value])),
        )
    }
    fn index_range_stream(&self, _: PhysicalIndexScan) -> SendableRecordBatchStream {
        unsupported_index()
    }
    fn text_search_stream(&self, _: PhysicalTextSearch) -> SendableRecordBatchStream {
        unsupported_index()
    }
}
