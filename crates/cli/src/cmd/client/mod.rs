use futures::future::BoxFuture;
use semantic_core::api;

use super::CliCommand;

mod create;
mod delete;
mod sql;

#[derive(clap::Parser, Clone, Debug)]
pub struct ClientOptions {
    /// Server address.
    ///
    /// Defaults to "http://localhost:3000".
    #[arg(short = 'a', long)]
    address: Option<url::Url>,
}

impl ClientOptions {
    fn build_client(&self) -> ReqwestApiClient {
        let endpoint = self
            .address
            .clone()
            .unwrap_or_else(|| "http://localhost:3000".parse().unwrap());
        ReqwestApiClient::new(ReqwestExecutor::new(endpoint))
    }
}

#[derive(clap::Subcommand)]
pub enum CmdClient {
    Delete(delete::CmdDelete),
    Sql(sql::CmdSql),
    Create(create::CmdClientCreate),
}

impl CliCommand for CmdClient {
    fn run(self) -> Result<(), anyhow::Error> {
        match self {
            Self::Delete(cmd) => cmd.run(),
            Self::Sql(cmd) => cmd.run(),
            Self::Create(cmd) => cmd.run(),
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
    type Future = BoxFuture<'static, Result<api::Reply, anyhow::Error>>;

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
