use std::io::Read as _;
use std::path::{Path, PathBuf};

use clap::Args;
use semantic_data::value::{Object, Value};
use semantic_rpc::{RpcClient, transport::http_client::HttpRpcClient};

use crate::CliError;

pub const DEFAULT_RPC_URL: &str = "http://127.0.0.1:8888/api/v1/rpc";

#[derive(Clone, Debug, Args)]
pub struct ApiClientArgs {
    /// Semantic HTTP RPC endpoint URL.
    ///
    /// May also be set with SEMANTIC_RPC_URL.
    #[arg(long, env = "SEMANTIC_RPC_URL", default_value = DEFAULT_RPC_URL, value_name = "URL")]
    pub rpc_url: String,

    /// Database scope ID for the request.
    ///
    /// May also be set with SEMANTIC_SCOPE; when omitted, the server chooses
    /// its default scope.
    #[arg(long, env = "SEMANTIC_SCOPE", value_name = "SCOPE_ID")]
    pub scope: Option<String>,
}

impl ApiClientArgs {
    pub fn http_client(&self) -> HttpRpcClient {
        HttpRpcClient::new(self.rpc_url.clone())
    }

    pub fn rpc_client(&self) -> RpcClient {
        self.http_client().into()
    }

    pub fn insert_scope(&self, payload: &mut Object) {
        if let Some(scope) = &self.scope {
            payload.insert("scope_id", Value::String(scope.clone()));
        }
    }
}

#[derive(Clone, Debug, Default, Args)]
pub struct CollectionArgs {
    /// Collection containing the affected records.
    ///
    /// When omitted, the server's default collection is used.
    #[arg(long, short = 'c', value_name = "COLLECTION")]
    pub collection: Option<String>,
}

impl CollectionArgs {
    pub fn insert_collection(&self, payload: &mut Object) {
        if let Some(collection) = &self.collection {
            payload.insert("collection", Value::String(collection.clone()));
        }
    }
}

#[derive(Clone, Debug, Default, Args)]
pub struct OutputArgs {
    /// Pretty-print the JSON response instead of emitting compact single-line JSON.
    #[arg(long)]
    pub pretty: bool,
}

impl OutputArgs {
    pub fn print(&self, value: &Value) -> std::result::Result<(), CliError> {
        let output = self.format(value)?;
        println!("{output}");
        Ok(())
    }

    fn format(&self, value: &Value) -> std::result::Result<String, CliError> {
        let result = if self.pretty {
            serde_json::to_string_pretty(value)
        } else {
            serde_json::to_string(value)
        };
        result.map_err(|source| CliError::Json {
            source_name: "API response".to_string(),
            source,
        })
    }
}

#[derive(Clone, Debug, Default, Args)]
pub struct FileInputArgs {
    /// UTF-8 input file; use '-' or omit INPUT to read from standard input.
    #[arg(value_name = "INPUT")]
    pub input: Option<PathBuf>,
}

impl FileInputArgs {
    pub fn read(&self) -> std::result::Result<String, CliError> {
        match self.input.as_deref() {
            None => read_stdin(),
            Some(path) if path == Path::new("-") => read_stdin(),
            Some(path) => std::fs::read_to_string(path).map_err(|source| CliError::Io {
                action: "read",
                path: path.display().to_string(),
                source,
            }),
        }
    }

    pub fn source_name(&self) -> String {
        self.input
            .as_deref()
            .filter(|path| *path != Path::new("-"))
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "standard input".to_string())
    }

    pub fn read_json(&self) -> std::result::Result<Value, CliError> {
        serde_json::from_str(&self.read()?).map_err(|source| CliError::Json {
            source_name: self.source_name(),
            source,
        })
    }

    pub fn read_json_object(&self) -> std::result::Result<Object, CliError> {
        match self.read_json()? {
            Value::Object(object) => Ok(object),
            _ => Err(CliError::InvalidInput(format!(
                "{} must contain a JSON object",
                self.source_name()
            ))),
        }
    }
}

#[derive(Clone, Debug, Default, Args)]
pub struct ConfirmationArgs {
    /// Authorize the destructive operation.
    ///
    /// No interactive prompt is shown; without this flag the command exits
    /// without making the request.
    #[arg(long, short = 'y')]
    pub yes: bool,
}

impl ConfirmationArgs {
    pub fn require(&self, target: impl Into<String>) -> std::result::Result<(), CliError> {
        if self.yes {
            Ok(())
        } else {
            Err(CliError::ConfirmationRequired {
                target: target.into(),
            })
        }
    }
}

fn read_stdin() -> std::result::Result<String, CliError> {
    let mut input = String::new();
    std::io::stdin()
        .read_to_string(&mut input)
        .map_err(|source| CliError::Io {
            action: "read",
            path: "standard input".to_string(),
            source,
        })?;
    Ok(input)
}

#[cfg(test)]
mod tests {
    use semantic_data::value::Value;

    use super::{ApiClientArgs, ConfirmationArgs, OutputArgs};

    #[test]
    fn api_client_inserts_scope_only_when_set() {
        let mut payload = semantic_data::value::Object::new();
        ApiClientArgs {
            rpc_url: "http://example.test/rpc".to_string(),
            scope: Some("research".to_string()),
        }
        .insert_scope(&mut payload);
        assert_eq!(
            payload.get("scope_id"),
            Some(&Value::String("research".to_string()))
        );
    }

    #[test]
    fn compact_output_is_single_line() {
        let value = Value::Object(
            [("answer".to_string(), Value::U64(42))]
                .into_iter()
                .collect(),
        );
        let output = OutputArgs { pretty: false }
            .format(&value)
            .expect("format output");
        assert_eq!(output, r#"{"answer":42}"#);
    }

    #[test]
    fn destructive_operations_require_explicit_confirmation() {
        let error = ConfirmationArgs { yes: false }
            .require("record-1")
            .expect_err("confirmation should be required");
        assert!(error.to_string().contains("--yes"));
        ConfirmationArgs { yes: true }
            .require("record-1")
            .expect("explicit confirmation");
    }
}
