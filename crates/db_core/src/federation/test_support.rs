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
