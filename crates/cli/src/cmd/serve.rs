use std::net::SocketAddr;

use super::{AppOptions, AsyncCliCommand};

/// Run the semantic server.
#[derive(clap::Parser)]
pub struct CmdServe {
    #[command(flatten)]
    app: AppOptions,

    /// The server interface to listen on.
    /// eg: `0.0.0.0:3000`
    #[arg(long, env = "SEMANTIC_ADDRESS")]
    address: Option<SocketAddr>,
}

impl AsyncCliCommand for CmdServe {
    async fn run(self) -> Result<(), anyhow::Error> {
        let app_config = self.app.build().unwrap();
        let config = semantic::server::ServerConfig {
            // Enable authentication when no backend is provided.
            require_auth: app_config.backend.is_none(),
            app: app_config,
            address: self
                .address
                .unwrap_or_else(|| "127.0.0.1:3000".parse().unwrap()),
        };

        let handle = tokio::runtime::Handle::current();
        semantic::server::run_server(config, handle).await?;

        Ok(())
    }
}
