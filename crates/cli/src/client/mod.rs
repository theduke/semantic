use factordb::AnyError;
use futures::future::BoxFuture;
use semantic_core::api;

mod delete;
mod sql;

#[derive(clap::Subcommand)]
pub(crate) enum ClientCommand {
    Delete(delete::DeleteCmd),
    Sql(sql::SqlCmd),
}

impl ClientCommand {
    pub fn run(self) {
        match self {
            Self::Delete(cmd) => {
                cmd.run();
            }
            Self::Sql(cmd) => {
                cmd.run();
            }
        }
    }
}

pub struct ReqwestExecutor {
    endpoint: url::Url,
    client: reqwest::Client,
}

impl ReqwestExecutor {
    pub fn new(endpoint: url::Url) -> Self {
        Self {
            endpoint,
            client: reqwest::Client::new(),
        }
    }
}

impl api::ApiClientExecutor for ReqwestExecutor {
    type Future = BoxFuture<'static, Result<api::Reply, AnyError>>;

    fn execute(&self, query: semantic_core::api::Query) -> Self::Future {
        let mut url = self.endpoint.clone();
        url.set_path("/api/query");

        let f = self
            .client
            .clone()
            .post(url.as_str().to_string())
            .json(&query)
            .send();

        Box::pin(async move {
            let res = f
                .await?
                .error_for_status()?
                .json::<api::ApiResponse>()
                .await?;
            match res {
                api::ApiResponse::Ok(r) => Ok(r),
                api::ApiResponse::Err(err) => {
                    anyhow::bail!("API error: {err:?}")
                }
            }
        })
    }
}

pub type ReqwestApiClient = api::ApiClient<ReqwestExecutor>;
