use bytes::Bytes;
use factordb::AnyError;
use futures::{future::BoxFuture, TryStream};
use semantic_core::{
    api::{self, ApiResponse, FileUploadMetadata},
    base::TypedFile,
};

#[derive(Clone, Debug)]
pub struct ApiClient {
    endpoint: reqwest::Url,

    client: reqwest::Client,
}

impl ApiClient {
    pub fn new(url: &str) -> Result<Self, anyhow::Error> {
        let mut endpoint: reqwest::Url = url.parse()?;
        endpoint.set_path("/");

        Ok(Self {
            endpoint,
            client: reqwest::Client::new(),
        })
    }

    pub async fn upload_file<S>(
        &self,
        meta: FileUploadMetadata,
        stream: S,
    ) -> Result<TypedFile, anyhow::Error>
    where
        S: TryStream + Send + Sync + 'static,
        S::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
        Bytes: From<S::Ok>,
    {
        let mut url = self.endpoint.clone();
        url.set_path("/api/upload-file");

        let header = base64::encode(serde_json::to_string(&meta)?);

        let body = reqwest::Body::wrap_stream(stream);

        let res = self
            .client
            .post(url)
            .header(api::FileUploadMetadata::HEADER_NAME, header)
            .body(body)
            .send()
            .await?
            .error_for_status()?
            .json::<ApiResponse<TypedFile>>()
            .await?;

        match res {
            ApiResponse::Ok(f) => Ok(f),
            ApiResponse::Err(err) => Err(anyhow::anyhow!("Upload failed: {}", err.message)),
        }
    }

    pub async fn upload_file_std(
        &self,
        meta: FileUploadMetadata,
        file: std::fs::File,
    ) -> Result<TypedFile, anyhow::Error> {
        let reader = tokio::io::BufReader::new(tokio::fs::File::from_std(file));
        let stream = tokio_util::io::ReaderStream::new(reader);
        self.upload_file(meta, stream).await
    }

    pub async fn upload_file_tokio(
        &self,
        meta: FileUploadMetadata,
        file: tokio::fs::File,
    ) -> Result<TypedFile, anyhow::Error> {
        let stream = tokio_util::io::ReaderStream::new(file);
        self.upload_file(meta, stream).await
    }

    fn api_endpoint(&self) -> reqwest::Url {
        let mut s = self.endpoint.clone();
        s.set_path("/api/query");
        s
    }
}

impl api::ApiClientExecutor for ApiClient {
    type Future = BoxFuture<'static, Result<api::Reply, AnyError>>;

    fn execute(&self, query: semantic_core::api::Query) -> Self::Future {
        let s = self.clone();
        Box::pin(async move {
            let reply = s
                .client
                .post(s.api_endpoint())
                .json(&query)
                .send()
                .await?
                .error_for_status()?
                .json::<api::ApiResponse<api::Reply>>()
                .await?;

            match reply {
                ApiResponse::Ok(r) => Ok(r),
                ApiResponse::Err(err) => Err(anyhow::anyhow!("API error: {}", err.message)),
            }
        })
    }
}
