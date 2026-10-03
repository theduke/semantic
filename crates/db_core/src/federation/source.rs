use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use semantic_data::query::DdlBatch;
use semantic_data::value::Value;
use semantic_data::vdb::{AcceptedScan, ScanPlan, ScanRequest};

use crate::{DbError, SendableRecordBatchStream};

/// A readable collection participating in a federated query.
#[async_trait]
pub trait QuerySource: Send + Sync + 'static {
    /// Decide which parts of the request this source applies, without side effects.
    async fn negotiate(&self, collection: &str, request: &ScanRequest)
    -> Result<ScanPlan, DbError>;

    /// Stream canonical entity rows according to the accepted plan.
    fn scan(self: Arc<Self>, scan: SourceScan) -> SendableRecordBatchStream;
}

#[derive(Clone, Debug)]
pub struct SourceScan {
    pub collection: String,
    pub request: ScanRequest,
    pub plan: AcceptedScan,
    pub bindings: BTreeMap<String, Value>,
}

/// Sources for one query. The local source serves every local collection.
#[derive(Clone)]
pub struct FederationSources {
    pub local: Arc<dyn QuerySource>,
    pub virtual_sources: BTreeMap<String, VirtualSource>,
}

/// A virtual collection and the runtime schema it exposes.
#[derive(Clone)]
pub struct VirtualSource {
    pub source: Arc<dyn QuerySource>,
    /// Validated, upsert-only DDL applied exclusively to the overlay catalog.
    pub schema: Arc<DdlBatch>,
    pub schema_revision: String,
}

/// A schema change asks the caller to refresh the virtual source and retry once.
#[derive(Debug, thiserror::Error)]
pub enum FederatedError {
    #[error("schema_changed: {collection}")]
    SchemaChanged { collection: String },
    #[error(transparent)]
    Db(#[from] DbError),
}

pub const LOCAL_SOURCE_TAG: &str = "local";
