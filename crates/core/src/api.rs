use std::collections::HashMap;

use factordb::{
    query::select::{Item, Page},
    AnyError,
};

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct SimpleHttpRequest {
    pub url: String,
    pub method: String,
    pub headers: HashMap<String, String>,
    pub body: Option<Vec<u8>>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct SimpleHttpResponse {
    pub status: u16,
    pub headers: HashMap<String, String>,
    pub body: Option<String>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct SemanticSchema {
    pub db: factordb::schema::DbSchema,
}

#[derive(serde::Serialize, serde::Deserialize, PartialEq, Eq, Clone)]
pub struct BackendCryptoConfig {
    pub data_path: Option<String>,
    pub key: String,
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
pub enum BackendConfig {
    Crypto(BackendCryptoConfig),
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct FileUploadMetadata {
    pub filename: Option<String>,
    pub title: Option<String>,
}

impl FileUploadMetadata {
    pub const HEADER_NAME: &'static str = "X-SEMANTIC-FILE-META";
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub enum Query {
    ServerStatus,
    Initialize {
        config: BackendConfig,
    },
    CloseBackend,

    Select(factordb::query::select::Select),
    Mutate(factordb::query::mutate::Mutate),
    Batch(factordb::query::mutate::BatchUpdate),

    Schema,

    Import {
        items: Vec<Item>,
        import_media: bool,
    },

    /// Execute an HTTP request.
    HttpFetch(SimpleHttpRequest),
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct QueryWithId {
    pub id: u64,
    pub query: Query,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct ServerStatus {
    pub backend_initialized: bool,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub enum Reply {
    ServerStatus(ServerStatus),
    Initialize,
    CloseBackend,

    Select(Page<Item>),
    Mutate,
    Batch,
    Schema(SemanticSchema),

    Import,
    HttpFetch(SimpleHttpResponse),
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct ApiError {
    pub message: String,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub enum ApiResponse<T = Reply> {
    Ok(T),
    Err(ApiError),
}

impl<T> ApiResponse<T> {
    pub fn from_res(res: Result<T, AnyError>) -> Self {
        match res {
            Ok(data) => Self::Ok(data),
            Err(err) => Self::Err(ApiError {
                message: err.to_string(),
            }),
        }
    }
}

pub trait ApiClientExecutor {
    type Future: std::future::Future<Output = Result<Reply, AnyError>>;
    fn execute(&self, query: Query) -> Self::Future;
}

#[derive(Clone)]
pub struct ApiClient<E: ApiClientExecutor> {
    exec: E,
}

impl<E: ApiClientExecutor> ApiClient<E> {
    pub fn new(exec: E) -> Self {
        Self { exec }
    }

    pub async fn server_status(&self) -> Result<ServerStatus, AnyError> {
        match self.exec.execute(Query::ServerStatus).await {
            Ok(Reply::ServerStatus(status)) => Ok(status),
            Ok(_other) => Err(anyhow::anyhow!("API returned invalid data")),
            Err(err) => Err(err),
        }
    }

    pub async fn initialize(&self, config: BackendConfig) -> Result<(), AnyError> {
        match self.exec.execute(Query::Initialize { config }).await {
            Ok(Reply::Initialize) => Ok(()),
            Ok(_other) => Err(anyhow::anyhow!("API returned invalid data")),
            Err(err) => Err(err),
        }
    }

    pub async fn close_backend(&self) -> Result<(), AnyError> {
        match self.exec.execute(Query::CloseBackend).await {
            Ok(Reply::CloseBackend) => Ok(()),
            Ok(_other) => Err(anyhow::anyhow!("API returned invalid data")),
            Err(err) => Err(err),
        }
    }

    pub async fn entity(&self, id: factordb::Id) -> Result<factordb::data::DataMap, AnyError> {
        use factordb::query::expr::Expr;
        let filter = Expr::eq(Expr::Attr("factor/id".into()), id);
        let mut page = self
            .select(factordb::query::select::Select::new().with_filter(filter))
            .await?;
        page.items
            .pop()
            .map(|x| x.data)
            .ok_or_else(|| anyhow::anyhow!("Not found"))
    }

    pub async fn select(
        &self,
        select: factordb::query::select::Select,
    ) -> Result<Page<Item>, AnyError> {
        match self.exec.execute(Query::Select(select)).await {
            Ok(Reply::Select(page)) => Ok(page),
            Ok(_other) => Err(anyhow::anyhow!("API returned invalid data")),
            Err(err) => Err(err),
        }
    }

    pub async fn select_entities<
        T,
    >(
        &self,
        select: factordb::query::select::Select,
    ) -> Result<Page<T>, AnyError> where T: factordb::schema::EntityContainer + serde::de::DeserializeOwned {
        match self.exec.execute(Query::Select(select)).await {
            Ok(Reply::Select(page)) => {
                let page2 = page.convert_data()?;
                Ok(page2)
            }
            Ok(_other) => Err(anyhow::anyhow!("API returned invalid data")),
            Err(err) => Err(err),
        }
    }

    pub async fn mutate(&self, mutate: factordb::query::mutate::Mutate) -> Result<(), AnyError> {
        match self.exec.execute(Query::Mutate(mutate)).await {
            Ok(Reply::Mutate) => Ok(()),
            Ok(_other) => Err(anyhow::anyhow!("API returned invalid data")),
            Err(err) => Err(err),
        }
    }

    pub async fn batch(&self, batch: factordb::query::mutate::BatchUpdate) -> Result<(), AnyError> {
        match self.exec.execute(Query::Batch(batch)).await {
            Ok(Reply::Batch) => Ok(()),
            Ok(_other) => Err(anyhow::anyhow!("API returned invalid data")),
            Err(err) => Err(err),
        }
    }

    pub async fn schema(&self) -> Result<SemanticSchema, AnyError> {
        match self.exec.execute(Query::Schema).await {
            Ok(Reply::Schema(schema)) => Ok(schema),
            Ok(_other) => Err(anyhow::anyhow!("API returned invalid data")),
            Err(err) => Err(err),
        }
    }

    pub async fn import(&self, items: Vec<Item>, import_media: bool) -> Result<(), AnyError> {
        match self
            .exec
            .execute(Query::Import {
                items,
                import_media,
            })
            .await
        {
            Ok(Reply::Import) => Ok(()),
            Ok(_other) => Err(anyhow::anyhow!("API returned invalid data")),
            Err(err) => Err(err),
        }
    }

    pub async fn http_fetch(
        &self,
        request: SimpleHttpRequest,
    ) -> Result<SimpleHttpResponse, AnyError> {
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
}
