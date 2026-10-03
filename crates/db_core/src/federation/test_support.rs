use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use futures::{StreamExt, stream};
use semantic_data::vdb::{AcceptedScan, FilterSupport, ScanPlan, ScanRequest};

use super::{QuerySource, SourceScan};
use crate::{DbError, SendableRecordBatchStream};

type Negotiator = dyn Fn(&ScanRequest) -> Result<ScanPlan, DbError> + Send + Sync;

pub(crate) struct TestSource {
    negotiate: Box<Negotiator>,
    requests: Mutex<Vec<(String, ScanRequest)>>,
}

impl TestSource {
    pub fn new(
        negotiate: impl Fn(&ScanRequest) -> Result<ScanPlan, DbError> + Send + Sync + 'static,
    ) -> Self {
        Self {
            negotiate: Box::new(negotiate),
            requests: Mutex::new(Vec::new()),
        }
    }

    pub fn requests(&self) -> Vec<(String, ScanRequest)> {
        self.requests.lock().unwrap().clone()
    }
}

#[async_trait]
impl QuerySource for TestSource {
    async fn negotiate(
        &self,
        collection: &str,
        request: &ScanRequest,
    ) -> Result<ScanPlan, DbError> {
        self.requests
            .lock()
            .unwrap()
            .push((collection.into(), request.clone()));
        (self.negotiate)(request)
    }

    fn scan(self: Arc<Self>, _: SourceScan) -> SendableRecordBatchStream {
        stream::empty().boxed()
    }
}

pub(crate) fn accepted(request: &ScanRequest) -> AcceptedScan {
    AcceptedScan {
        filters: vec![FilterSupport::Exact; request.filters.len()],
        ordered_prefix: request.order_by.len() as u64,
        limit_applied: request.limit.is_some(),
        offset_applied: request.offset != 0,
        estimated_rows: None,
        token: None,
        schema_revision: "1".into(),
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum NegotiationMode {
    Exact,
    Inexact,
    Unsupported,
    Alternating,
}

pub(crate) struct MemorySource {
    pub rows: std::collections::BTreeMap<String, Vec<semantic_data::value::Object>>,
    pub mode: NegotiationMode,
    pub revision: String,
}

#[async_trait]
impl QuerySource for MemorySource {
    async fn negotiate(
        &self,
        collection: &str,
        request: &ScanRequest,
    ) -> Result<ScanPlan, DbError> {
        let mut plan = accepted(request);
        plan.filters = request
            .filters
            .iter()
            .enumerate()
            .map(|(index, _)| match self.mode {
                NegotiationMode::Exact => FilterSupport::Exact,
                NegotiationMode::Inexact => FilterSupport::Inexact,
                NegotiationMode::Unsupported => FilterSupport::Unsupported,
                NegotiationMode::Alternating => {
                    if index % 2 == 0 {
                        FilterSupport::Exact
                    } else {
                        FilterSupport::Inexact
                    }
                }
            })
            .collect();
        if matches!(
            self.mode,
            NegotiationMode::Inexact | NegotiationMode::Unsupported
        ) {
            plan.ordered_prefix = 0;
        }
        let can_limit = plan.all_exact() && plan.ordered_prefix == request.order_by.len() as u64;
        plan.limit_applied = can_limit && request.limit.is_some();
        plan.offset_applied = can_limit && request.offset != 0;
        plan.schema_revision = self.revision.clone();
        plan.estimated_rows = Some(self.rows.get(collection).map_or(0, Vec::len) as u64);
        Ok(ScanPlan::Accepted { plan })
    }

    fn scan(self: Arc<Self>, scan: SourceScan) -> SendableRecordBatchStream {
        let mut rows = self.rows.get(&scan.collection).cloned().unwrap_or_default();
        for (filter, support) in scan.request.filters.iter().zip(&scan.plan.filters) {
            if *support == FilterSupport::Exact {
                let filter: crate::Expr = filter.clone().into();
                rows.retain(|row| crate::evaluate_filter_expr(row, &filter));
            }
        }
        let order = scan
            .request
            .order_by
            .into_iter()
            .take(scan.plan.ordered_prefix as usize)
            .map(Into::into)
            .collect::<Vec<crate::OrderBy>>();
        if !order.is_empty() {
            rows.sort_by(|left, right| crate::query::compare_objects_for_plan(left, right, &order));
        }
        if scan.plan.offset_applied {
            rows = rows
                .into_iter()
                .skip(scan.request.offset as usize)
                .collect();
        }
        if scan.plan.limit_applied {
            rows.truncate(scan.request.limit.unwrap() as usize);
        }
        stream::iter(
            rows.into_iter()
                .map(|row| Ok(vec![Box::new(row) as crate::DynObject])),
        )
        .boxed()
    }
}
