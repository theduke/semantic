use std::{collections::HashMap, ops::Deref};

use factdb::{ClassContainer, DataMap, Id, IdOrIdent, Mutate, Timestamp};
use url::Url;

use crate::{
    base::RecordEntityVisit,
    core::PluginSource,
    plugin::{FetchUrlJob, FetchUrlOutput, ImportJob, ImportOutput},
};

pub const DEFAULT_PORT: u16 = 3000;

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", derive(ts_rs::TS))]
pub struct SimpleHttpRequest {
    pub url: String,
    pub method: String,
    pub headers: HashMap<String, String>,
    pub body: Option<Vec<u8>>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", derive(ts_rs::TS))]
pub struct SimpleHttpResponse {
    pub status: u16,
    pub headers: HashMap<String, String>,
    pub body: Option<String>,
}

#[derive(serde::Serialize, serde::Deserialize, Default, Clone, Debug)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", derive(ts_rs::TS))]
pub struct SemanticSchema {
    pub db: factdb::schema::DbSchema,
}

#[derive(serde::Serialize, serde::Deserialize, PartialEq, Eq, Clone)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", derive(ts_rs::TS))]
pub struct BackendCryptoConfig {
    pub data_path: Option<String>,
    pub key: String,
    pub key_iterations: Option<u32>,
    pub salt: Option<String>,
    pub raw: bool,
    pub offset: Option<u64>,
    pub full_index_write_interval: Option<u64>,
    pub readonly: bool,
}

impl std::fmt::Debug for BackendCryptoConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BackendCryptoConfig")
            .field("data_path", &self.data_path)
            .field("key", &"*****")
            .finish()
    }
}

#[derive(serde::Serialize, serde::Deserialize, PartialEq, Eq, Clone, Debug)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", derive(ts_rs::TS))]
pub enum DbConfig {
    InMemory,
    Crypto(BackendCryptoConfig),
    BlobFs(BlobFsConfig),
}

#[derive(serde::Serialize, serde::Deserialize, PartialEq, Eq, Clone, Debug)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", derive(ts_rs::TS))]
pub struct BlobFsConfig {
    pub path: String,
    pub password: String,
}

impl DbConfig {
    /// Remove all sensitive information like passwords.
    pub fn purge_secrets(self) -> Self {
        match self {
            Self::InMemory => Self::InMemory,
            Self::Crypto(c) => Self::Crypto(BackendCryptoConfig {
                offset: c.offset,
                data_path: c.data_path,
                salt: None,
                key_iterations: None,
                key: String::new(),
                raw: false,
                full_index_write_interval: c.full_index_write_interval,
                readonly: c.readonly,
            }),
            DbConfig::BlobFs(mut b) => {
                // TODO: actually overwrite.
                b.password.clear();
                Self::BlobFs(b)
            }
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", derive(ts_rs::TS))]
pub struct BackendConfig {
    pub db: DbConfig,
    pub idle_timeout: Option<Seconds>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, Default)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", derive(ts_rs::TS))]
pub struct FileUploadMetadata {
    pub filename: Option<String>,
    pub title: Option<String>,
    #[cfg_attr(feature = "schema", ts(type = "string|null|undefined"))]
    pub url: Option<url::Url>,
    pub ident: Option<String>,
    pub parent: Option<Id>,
    /// Sort order for the parent. (attribute semantic/parent_sort_order)
    pub parent_sort: Option<i64>,
    /// Id of the collection to which the uploaded files should be added.
    pub collection_id: Option<Id>,
    #[serde(default)]
    pub tag_ids: Vec<Id>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, Default)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", derive(ts_rs::TS))]
pub struct FileUploadReply {
    pub file: DataMap,
    pub is_new: bool,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct FileImportMetadata {
    /// Id of the collection to which the uploaded files should be added.
    pub collection_id: Option<Id>,
    pub url: Option<url::Url>,
    #[serde(default)]
    pub tags: Vec<IdOrIdent>,
    pub parent: Option<Id>,
}

impl FileUploadMetadata {
    pub const HEADER_NAME: &'static str = "X-SEMANTIC-FILE-META";
}

pub type Seconds = u64;

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", derive(ts_rs::TS))]
pub struct PluginTestFetch {
    pub runtime: String,
    pub code: String,
    #[cfg_attr(feature = "schema", ts(type = "string"))]
    pub url: Url,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", derive(ts_rs::TS))]
pub enum JobStatus {
    Queued {
        queue_position: Option<u64>,
    },
    Running {
        step: Option<String>,
        progress_percent: Option<f64>,
        progress_message: Option<String>,
    },
    Finished {
        #[cfg_attr(feature = "schema", ts(type = "{Ok: string} | {Err: ApiError}"))]
        result: Result<String, ApiError>,
    },
}

impl JobStatus {
    /// Returns `true` if the job status is [`Finished`].
    ///
    /// [`Finished`]: JobStatus::Finished
    pub fn is_finished(&self) -> bool {
        matches!(self, Self::Finished { .. })
    }

    pub fn failed_from_anyhow(err: &anyhow::Error) -> Self {
        Self::Finished {
            result: Err(ApiError::from_error(err)),
        }
    }
}

pub type JobId = uuid::Uuid;

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", derive(ts_rs::TS))]
pub struct JobStep {
    pub name: String,
    pub started_at: Option<Timestamp>,
    pub finished_at: Option<Timestamp>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", derive(ts_rs::TS))]
pub struct Job {
    pub id: JobId,
    pub name: String,
    pub created_at: Timestamp,
    pub started_at: Option<Timestamp>,
    pub finished_at: Option<Timestamp>,
    pub steps: Vec<JobStep>,
    pub status: JobStatus,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", derive(ts_rs::TS))]
pub struct JobEvent {
    pub name: String,
    #[cfg_attr(feature = "schema", ts(type = "any"))]
    pub data: serde_json::Value,
}

impl Job {
    pub fn update(&mut self, status: JobStatus) {
        let now = Timestamp::now();

        match &status {
            JobStatus::Queued { queue_position: _ } => {}
            JobStatus::Running {
                step,
                progress_message: _,
                progress_percent: _,
            } => {
                if self.started_at.is_none() {
                    self.started_at = Some(now);
                }

                if let Some(step_name) = step.as_ref() {
                    // Mark old step as finished.
                    match &self.status {
                        JobStatus::Running {
                            step: Some(old_step),
                            progress_message: _,
                            progress_percent: _,
                        } if old_step != step_name => {
                            if let Some(old_step) =
                                self.steps.iter_mut().find(|s| &s.name == old_step)
                            {
                                old_step.finished_at = Some(now);
                            }
                        }
                        _ => {}
                    }

                    let index = self
                        .steps
                        .iter_mut()
                        .enumerate()
                        .find(|(_index, s)| &s.name == step_name)
                        .map(|(index, _)| index)
                        .unwrap_or_else(|| {
                            self.steps.push(JobStep {
                                name: step_name.to_string(),
                                started_at: Some(now),
                                finished_at: None,
                            });
                            self.steps.len() - 1
                        });

                    let step = self.steps.get_mut(index).unwrap();
                    step.started_at = Some(now);
                }
            }
            JobStatus::Finished { .. } => {
                self.finished_at = Some(now);
            }
        }

        self.status = status;
    }
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", derive(ts_rs::TS))]
pub struct ConvertFile {
    pub file_id: Id,
    pub target_format: String,
    #[cfg_attr(feature = "schema", ts(type = "any | null | undefined"))]
    pub settings: Option<serde_json::Value>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", derive(ts_rs::TS))]
pub struct OptimiseVideo {
    pub video_id: Id,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", derive(ts_rs::TS))]
pub struct OptimiseVideoReply {
    pub job_id: JobId,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", derive(ts_rs::TS))]
pub struct FileCreatePreviewImageBlob {
    pub file_id: Id,
    /// base64 encoded image content
    pub data: String,
    pub mime_type: String,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", derive(ts_rs::TS))]
pub struct PluginDelete {
    pub name: String,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", derive(ts_rs::TS))]
pub struct FileDiscardUnOptimized {
    pub file_id: Id,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", derive(ts_rs::TS))]
pub struct TagCreate {
    pub name: String,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", derive(ts_rs::TS))]
pub struct SimilarImageOptions {
    pub max_results: u64,
    pub similarity_min: f32,
    pub similarity_max: f32,
}

/// Merge a source tag into a target tag.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", derive(ts_rs::TS))]
pub struct TagMerge {
    pub target_tag: IdOrIdent,
    pub source_tag: IdOrIdent,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", derive(ts_rs::TS))]
pub struct QuerySql {
    pub query: String,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", derive(ts_rs::TS))]
pub enum Query {
    ServerStatus(()),
    Initialize(BackendConfig),
    CloseBackend(()),

    Select(factdb::query::select::Select),
    QuerySql(QuerySql),
    Mutate(factdb::query::mutate::Mutate),
    Batch(factdb::query::mutate::Batch),

    Schema(()),

    PluginSourceCreate(PluginSource),
    PluginSourceUpdate(PluginSource),
    PluginSourceValidate(PluginSource),
    PluginDelete(PluginDelete),
    PluginTestFetch(PluginTestFetch),

    TagCreate(TagCreate),
    TagMerge(TagMerge),

    Import(ImportJob),
    ImportRaw(ImportRaw),
    FetchUrl(FetchUrlJob),
    OptimiseVideo(OptimiseVideo),
    FileDiscardUnOptimized(FileDiscardUnOptimized),
    FileDiscardOptimised(FileDiscardUnOptimized),
    FileCreatePreviewImageBlob(FileCreatePreviewImageBlob),

    RecordEntityVisit(RecordEntityVisit),

    /// Execute an HTTP request.
    HttpFetch(SimpleHttpRequest),

    JobStatus(JobId),
    JobEvents(JobId),

    ConvertFile(ConvertFile),

    FindUnusedBlobs(()),
    DeleteUnusedBlobs(()),
    AnalyzeMedia {
        force: bool,
    },
    FindSimilarImages(SimilarImageOptions),
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", derive(ts_rs::TS))]
pub struct ImportRaw {
    #[cfg_attr(feature = "schema", ts(type = "any"))]
    pub data: serde_json::Value,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct QueryWithId {
    pub id: u64,
    pub query: Query,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", derive(ts_rs::TS))]
pub struct ServerStatus {
    pub backend_initialized: bool,
    pub backend_status: Option<BackendStatus>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", derive(ts_rs::TS))]
pub struct BackendStatus {
    /// Size of database data in kilobytes.
    pub db_size: Option<u64>,
    /// Size of assets/files in kilobytes.
    pub asset_size: Option<u64>,
    /// Full size of disk storage.
    pub storage_size: Option<u64>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", derive(ts_rs::TS))]
pub struct BlobInfo {
    pub key: String,
    pub size: u64,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", derive(ts_rs::TS))]
pub struct UnusedBlobsDeleted {
    pub count: u64,
    pub reclaimed_size: u64,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", derive(ts_rs::TS))]
pub struct ImportRawReply {
    pub items: Vec<DataMap>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", derive(ts_rs::TS))]
pub enum Reply {
    ServerStatus(ServerStatus),
    Initialize(SemanticSchema),
    CloseBackend(()),

    Select(Vec<DataMap>),
    QuerySql(Vec<DataMap>),

    Mutate(()),
    Batch(()),
    Schema(SemanticSchema),

    PluginSourceCreate(PluginSource),
    PluginSourceUpgrade(PluginSource),
    PluginSourceValidate(()),
    PluginDelete(()),
    PluginTestFetch(Option<FetchUrlOutput>),

    TagCreate(DataMap),
    TagMerge(DataMap),

    Import(ImportOutput),
    ImportRaw(ImportRawReply),
    FetchUrl(FetchUrlOutput),
    HttpFetch(SimpleHttpResponse),
    OptimiseVideo(OptimiseVideoReply),
    FileDiscardOptimised(()),
    FileDiscardUnOptimised(()),
    FileCreatePreviewImageBlob(()),

    RecordEntityVisit,

    JobStatus(Job),
    JobEvents(Vec<JobEvent>),
    ConvertFile(Job),

    FindUnusedBlobs { items: Vec<BlobInfo> },
    DeleteUnusedBlobs(UnusedBlobsDeleted),
    AnalyzeMedia(Job),
    FindSimilarImages(Job),
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", derive(ts_rs::TS))]
pub struct ApiError {
    pub message: String,
    pub code: Option<String>,
    pub details: Option<String>,
}

impl ApiError {
    pub fn from_error(err: &anyhow::Error) -> Self {
        Self {
            message: err.to_string(),
            code: None,
            details: None,
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", derive(ts_rs::TS))]
pub enum ApiResponse<T = Reply> {
    Ok(T),
    Err(ApiError),
}

impl<T> ApiResponse<T> {
    pub fn from_res(res: Result<T, anyhow::Error>) -> Self {
        match res {
            Ok(data) => Self::Ok(data),
            Err(err) => Self::Err(ApiError {
                message: err.to_string(),
                code: None,
                details: Some(format!("{:?}", err)),
            }),
        }
    }
}

pub trait ApiClientExecutor {
    type Future: std::future::Future<Output = Result<Reply, anyhow::Error>>;
    fn execute(&self, query: Query) -> Self::Future;
}

#[derive(Clone)]
pub struct ApiClient<E: ApiClientExecutor> {
    exec: E,
}

impl<E: ApiClientExecutor> Deref for ApiClient<E> {
    type Target = E;

    fn deref(&self) -> &Self::Target {
        &self.exec
    }
}

impl<E: ApiClientExecutor> ApiClient<E> {
    pub fn new(exec: E) -> Self {
        Self { exec }
    }

    pub async fn server_status(&self) -> Result<ServerStatus, anyhow::Error> {
        tracing::trace!("executing server status");
        let res = self.exec.execute(Query::ServerStatus(())).await;
        tracing::trace!("got server_status res");
        match res {
            Ok(Reply::ServerStatus(status)) => Ok(status),
            Ok(_other) => Err(anyhow::anyhow!("API returned invalid data")),
            Err(err) => Err(err),
        }
    }

    pub async fn initialize(
        &self,
        options: BackendConfig,
    ) -> Result<SemanticSchema, anyhow::Error> {
        match self.exec.execute(Query::Initialize(options)).await {
            Ok(Reply::Initialize(schema)) => Ok(schema),
            Ok(_other) => Err(anyhow::anyhow!("API returned invalid data")),
            Err(err) => Err(err),
        }
    }

    pub async fn close_backend(&self) -> Result<(), anyhow::Error> {
        match self.exec.execute(Query::CloseBackend(())).await {
            Ok(Reply::CloseBackend(_)) => Ok(()),
            Ok(_other) => Err(anyhow::anyhow!("API returned invalid data")),
            Err(err) => Err(err),
        }
    }

    pub async fn entity(&self, id: Id) -> Result<factdb::data::DataMap, anyhow::Error> {
        use factdb::query::expr::Expr;
        let filter = Expr::eq(Expr::Attr("factor/id".into()), id);
        let mut items = self
            .select(factdb::query::select::Select::new().with_filter(filter))
            .await?;
        items.pop().ok_or_else(|| anyhow::anyhow!("Not found"))
    }

    pub async fn select(
        &self,
        select: factdb::query::select::Select,
    ) -> Result<Vec<DataMap>, anyhow::Error> {
        match self.exec.execute(Query::Select(select)).await {
            Ok(Reply::Select(items)) => Ok(items),
            Ok(_other) => Err(anyhow::anyhow!("API returned invalid data")),
            Err(err) => Err(err),
        }
    }

    pub async fn select_sql(&self, query: String) -> Result<Vec<DataMap>, anyhow::Error> {
        match self.exec.execute(Query::QuerySql(QuerySql { query })).await {
            Ok(Reply::QuerySql(items)) => Ok(items),
            Ok(_other) => Err(anyhow::anyhow!("API returned invalid data")),
            Err(err) => Err(err),
        }
    }

    pub async fn select_entities<T>(
        &self,
        select: factdb::query::select::Select,
    ) -> Result<Vec<T>, anyhow::Error>
    where
        T: factdb::schema::ClassContainer + serde::de::DeserializeOwned,
    {
        match self.exec.execute(Query::Select(select)).await {
            Ok(Reply::Select(items)) => items
                .into_iter()
                .map(|map| T::try_from_map(map).map_err(anyhow::Error::from))
                .collect::<Result<Vec<_>, _>>(),
            Ok(_other) => Err(anyhow::anyhow!("API returned invalid data")),
            Err(err) => Err(err),
        }
    }

    pub async fn mutate(&self, mutate: factdb::query::mutate::Mutate) -> Result<(), anyhow::Error> {
        match self.exec.execute(Query::Mutate(mutate)).await {
            Ok(Reply::Mutate(())) => Ok(()),
            Ok(_other) => Err(anyhow::anyhow!("API returned invalid data")),
            Err(err) => Err(err),
        }
    }

    pub async fn entity_create<V: ClassContainer + serde::Serialize>(
        &self,
        entity: V,
    ) -> Result<(), anyhow::Error> {
        let id = entity.id();
        let data = entity.into_map()?;
        self.mutate(Mutate::create(id, data)).await
    }

    pub async fn batch(&self, batch: factdb::query::mutate::Batch) -> Result<(), anyhow::Error> {
        match self.exec.execute(Query::Batch(batch)).await {
            Ok(Reply::Batch(())) => Ok(()),
            Ok(_other) => Err(anyhow::anyhow!("API returned invalid data")),
            Err(err) => Err(err),
        }
    }

    pub async fn schema(&self) -> Result<SemanticSchema, anyhow::Error> {
        match self.exec.execute(Query::Schema(())).await {
            Ok(Reply::Schema(schema)) => Ok(schema),
            Ok(_other) => Err(anyhow::anyhow!("API returned invalid data")),
            Err(err) => Err(err),
        }
    }

    pub async fn plugin_source_create(
        &self,
        source: PluginSource,
    ) -> Result<PluginSource, anyhow::Error> {
        match self.exec.execute(Query::PluginSourceCreate(source)).await {
            Ok(Reply::PluginSourceCreate(source)) => Ok(source),
            Ok(_other) => Err(anyhow::anyhow!("API returned invalid data")),
            Err(err) => Err(err),
        }
    }

    pub async fn plugin_source_validate(&self, source: PluginSource) -> Result<(), anyhow::Error> {
        match self.exec.execute(Query::PluginSourceValidate(source)).await {
            Ok(Reply::PluginSourceValidate(())) => Ok(()),
            Ok(_other) => Err(anyhow::anyhow!("API returned invalid data")),
            Err(err) => Err(err),
        }
    }

    pub async fn plugin_source_upgrade(
        &self,
        source: PluginSource,
    ) -> Result<PluginSource, anyhow::Error> {
        match self.exec.execute(Query::PluginSourceUpdate(source)).await {
            Ok(Reply::PluginSourceUpgrade(source)) => Ok(source),
            Ok(_other) => Err(anyhow::anyhow!("API returned invalid data")),
            Err(err) => Err(err),
        }
    }

    pub async fn plugin_delete(&self, name: String) -> Result<(), anyhow::Error> {
        match self
            .exec
            .execute(Query::PluginDelete(PluginDelete { name }))
            .await
        {
            Ok(Reply::PluginDelete(())) => Ok(()),
            Ok(_other) => Err(anyhow::anyhow!("API returned invalid data")),
            Err(err) => Err(err),
        }
    }

    pub async fn plugin_test_fetch(
        &self,
        spec: PluginTestFetch,
    ) -> Result<Option<FetchUrlOutput>, anyhow::Error> {
        match self.exec.execute(Query::PluginTestFetch(spec)).await {
            Ok(Reply::PluginTestFetch(out)) => Ok(out),
            Ok(_other) => Err(anyhow::anyhow!("API returned invalid data")),
            Err(err) => Err(err),
        }
    }

    pub async fn fetch_url(&self, job: FetchUrlJob) -> Result<FetchUrlOutput, anyhow::Error> {
        match self.exec.execute(Query::FetchUrl(job)).await {
            Ok(Reply::FetchUrl(output)) => Ok(output),
            Ok(_other) => Err(anyhow::anyhow!("API returned invalid data")),
            Err(err) => Err(err),
        }
    }

    pub async fn import(&self, job: ImportJob) -> Result<ImportOutput, anyhow::Error> {
        match self.exec.execute(Query::Import(job)).await {
            Ok(Reply::Import(output)) => Ok(output),
            Ok(_other) => Err(anyhow::anyhow!("API returned invalid data")),
            Err(err) => Err(err),
        }
    }

    pub async fn job(&self, job_id: JobId) -> Result<Job, anyhow::Error> {
        match self.exec.execute(Query::JobStatus(job_id)).await {
            Ok(Reply::JobStatus(output)) => Ok(output),
            Ok(_other) => Err(anyhow::anyhow!("API returned invalid data")),
            Err(err) => Err(err),
        }
    }

    pub async fn optimise_video(
        &self,
        job: OptimiseVideo,
    ) -> Result<OptimiseVideoReply, anyhow::Error> {
        match self.exec.execute(Query::OptimiseVideo(job)).await {
            Ok(Reply::OptimiseVideo(output)) => Ok(output),
            Ok(_other) => Err(anyhow::anyhow!("API returned invalid data")),
            Err(err) => Err(err),
        }
    }

    pub async fn file_discard_optimized(&self, file_id: Id) -> Result<(), anyhow::Error> {
        match self
            .exec
            .execute(Query::FileDiscardOptimised(FileDiscardUnOptimized {
                file_id,
            }))
            .await
        {
            Ok(Reply::FileDiscardOptimised(())) => Ok(()),
            Ok(_other) => Err(anyhow::anyhow!("API returned invalid data")),
            Err(err) => Err(err),
        }
    }

    pub async fn file_discard_un_optimized(&self, file_id: Id) -> Result<(), anyhow::Error> {
        match self
            .exec
            .execute(Query::FileDiscardUnOptimized(FileDiscardUnOptimized {
                file_id,
            }))
            .await
        {
            Ok(Reply::FileDiscardUnOptimised(())) => Ok(()),
            Ok(_other) => Err(anyhow::anyhow!("API returned invalid data")),
            Err(err) => Err(err),
        }
    }

    pub async fn find_unused_blobs(&self) -> Result<Vec<BlobInfo>, anyhow::Error> {
        match self.exec.execute(Query::FindUnusedBlobs(())).await {
            Ok(Reply::FindUnusedBlobs { items }) => Ok(items),
            Ok(_other) => Err(anyhow::anyhow!("API returned invalid data")),
            Err(err) => Err(err),
        }
    }

    pub async fn delete_unused_blobs(&self) -> Result<UnusedBlobsDeleted, anyhow::Error> {
        match self.exec.execute(Query::DeleteUnusedBlobs(())).await {
            Ok(Reply::DeleteUnusedBlobs(info)) => Ok(info),
            Ok(_other) => Err(anyhow::anyhow!("API returned invalid data")),
            Err(err) => Err(err),
        }
    }

    pub async fn analyze_media(&self, force: bool) -> Result<Job, anyhow::Error> {
        match self.exec.execute(Query::AnalyzeMedia { force }).await {
            Ok(Reply::AnalyzeMedia(job)) => Ok(job),
            Ok(_other) => Err(anyhow::anyhow!("API returned invalid data")),
            Err(err) => Err(err),
        }
    }

    pub async fn file_create_preview_image_blob(
        &self,
        data: FileCreatePreviewImageBlob,
    ) -> Result<(), anyhow::Error> {
        match self
            .exec
            .execute(Query::FileCreatePreviewImageBlob(data))
            .await
        {
            Ok(Reply::FileCreatePreviewImageBlob(())) => Ok(()),
            Ok(_other) => Err(anyhow::anyhow!("API returned invalid data")),
            Err(err) => Err(err),
        }
    }

    pub async fn tag_create(&self, create: TagCreate) -> Result<DataMap, anyhow::Error> {
        match self.exec.execute(Query::TagCreate(create)).await {
            Ok(Reply::TagCreate(tag)) => Ok(tag),
            Ok(_other) => Err(anyhow::anyhow!("API returned invalid data")),
            Err(err) => Err(err),
        }
    }

    pub async fn tag_merge(
        &self,
        source_tag: IdOrIdent,
        target_tag: IdOrIdent,
    ) -> Result<DataMap, anyhow::Error> {
        match self
            .exec
            .execute(Query::TagMerge(TagMerge {
                target_tag,
                source_tag,
            }))
            .await
        {
            Ok(Reply::TagMerge(new_tag)) => Ok(new_tag),
            Ok(_other) => Err(anyhow::anyhow!("API returned invalid data")),
            Err(err) => Err(err),
        }
    }

    pub async fn http_fetch(
        &self,
        request: SimpleHttpRequest,
    ) -> Result<SimpleHttpResponse, anyhow::Error> {
        match self.exec.execute(Query::HttpFetch(request)).await {
            Ok(Reply::HttpFetch(mut response)) => {
                let body = if let Some(body) = response.body {
                    let decoded = base64::decode(&body)?;
                    let s = String::from_utf8(decoded)?;
                    Some(s)
                } else {
                    None
                };
                response.body = body;
                Ok(response)
            }
            Ok(_other) => Err(anyhow::anyhow!("API returned invalid data")),
            Err(err) => Err(err),
        }
    }

    pub async fn record_entity_visit(&self, entity_id: Id) -> Result<(), anyhow::Error> {
        let visit = RecordEntityVisit {
            entity_id,
            time: None,
        };
        match self.exec.execute(Query::RecordEntityVisit(visit)).await {
            Ok(Reply::RecordEntityVisit) => Ok(()),
            Ok(_other) => Err(anyhow::anyhow!("API returned invalid data")),
            Err(err) => Err(err),
        }
    }
}
