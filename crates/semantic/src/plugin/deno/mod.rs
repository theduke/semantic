use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
};

use anyhow::Context;
use factordb::AnyError;
use semantic_core::plugin::{ImportMatch, ImportOutput, PluginSchema};
use sha2::Digest;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    sync::Mutex,
};

const BRIDGE_CODE: &'static str = include_str!("./bridge.ts");

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct DenoConfig {
    pub data_dir: PathBuf,
    pub plugin_dir: Option<PathBuf>,
}

impl DenoConfig {
    fn bridge_path(&self) -> PathBuf {
        self.data_dir.join("bridges")
    }

    fn ensure_bridge_dir(&self) -> Result<PathBuf, AnyError> {
        let p = self.bridge_path();
        if !p.is_dir() {
            std::fs::create_dir_all(&p)?;
        }
        Ok(p)
    }
}

#[derive(Clone, Debug)]
struct PluginData {
    plugin_file: PathBuf,
    schema: PluginSchema,
    code: String,
}

struct State {
    config: DenoConfig,
    plugins: HashMap<String, PluginData>,
    workers: HashMap<String, Arc<Mutex<Worker>>>,
}

struct Worker {
    child: tokio::process::Child,
    stdin: tokio::process::ChildStdin,
    // stdout: tokio::process::ChildStdout,
    stderr: BufReader<tokio::process::ChildStderr>,
}

impl Worker {
    // bridge_path is the path to the bridge.ts file.
    async fn boot(bridge_path: &Path) -> Result<Self, AnyError> {
        let mut child = tokio::process::Command::new("deno")
            .arg("run")
            // FIXME: use restrictive permissions
            .arg("-A")
            .stdin(std::process::Stdio::piped())
            // .stdout(std::process::Stdio::piped())
            .stdout(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::piped())
            // Read code from stdin
            .arg(bridge_path)
            .spawn()?;

        // let stdout = child.stdout.take().context("Child does not have stdout")?;
        let stdin = child.stdin.take().context("Child does not have stdin")?;
        let stderr = child.stderr.take().context("Child does not have stderr")?;

        let w = Self {
            child,
            stdin,
            // stdout,
            stderr: BufReader::new(stderr),
        };

        Ok(w)
    }

    async fn send_command(&mut self, command: PluginCommand) -> Result<PluginReply, AnyError> {
        // FIXME: taint and destroy workers if they fail internally so they can
        // be re-created. Probably want a WorkerManager that handles this.
        let mut cmd_raw = serde_json::to_vec(&command)?;
        cmd_raw.push(b'\n');
        self.stdin.write_all(&cmd_raw).await?;

        let mut buffer = Vec::new();
        self.stderr.read_until(b'\n', &mut buffer).await?;

        crate::util::json_from_slice::<Result<PluginReply, String>>(&buffer)?
            .map_err(|err| AnyError::msg(err))
    }

    async fn send_init(&mut self, plugin_path: &Path) -> Result<PluginSchema, AnyError> {
        let plugin_path = plugin_path.to_str().context("invalid path")?.to_string();
        match self
            .send_command(PluginCommand::Init { plugin_path })
            .await?
        {
            PluginReply::Init { schema } => Ok(schema),
            _other => Err(anyhow::anyhow!("Plugin sent invalid response")),
        }
    }

    async fn send_ping(&mut self) -> Result<(), AnyError> {
        match self.send_command(PluginCommand::Ping).await? {
            PluginReply::Ping => Ok(()),
            _other => Err(anyhow::anyhow!("Plugin sent invalid response")),
        }
    }

    async fn send_import(&mut self, url: &url::Url) -> Result<Option<ImportOutput>, AnyError> {
        match self
            .send_command(PluginCommand::Import {
                url: url.to_string(),
            })
            .await?
        {
            PluginReply::Import { output } => Ok(output),
            _other => Err(anyhow::anyhow!("Plugin sent invalid response")),
        }
    }
}

#[derive(Clone)]
pub struct DenoPluginHost {
    state: Arc<RwLock<State>>,
}

impl DenoPluginHost {
    pub async fn start(config: DenoConfig) -> Result<Self, AnyError> {
        if !config.data_dir.is_dir() {
            std::fs::create_dir_all(&config.data_dir)?;
        }
        config.ensure_bridge_dir()?;

        let mut plugins = HashMap::new();
        let mut workers = HashMap::new();

        match config.plugin_dir.as_ref() {
            Some(dir) => match Self::load_dir(dir, &config).await {
                Ok(items) => {
                    for (data, worker) in items {
                        workers.insert(data.schema.name.clone(), Arc::new(Mutex::new(worker)));
                        plugins.insert(data.schema.name.clone(), data);
                    }
                }
                Err(err) => {
                    tracing::error!(?err, "Could not load deno plugins",);
                }
            },
            None => {}
        };

        let host = Self {
            state: Arc::new(RwLock::new(State {
                plugins,
                workers,
                config,
            })),
        };

        Ok(host)
    }

    pub fn plugins(&self) -> Vec<PluginSchema> {
        self.state
            .read()
            .unwrap()
            .plugins
            .values()
            .map(|x| x.schema.clone())
            .collect()
    }

    fn find_importer(&self, url: &url::Url) -> Option<(PluginData, ImportMatch)> {
        self.state
            .read()
            .unwrap()
            .plugins
            .values()
            .filter_map(|plugin| {
                let m = plugin.schema.find_import_match(url)?;
                Some((plugin.clone(), m))
            })
            .max_by(|a, b| a.1.support.cmp(&b.1.support))
    }

    pub async fn import(&self, url: &url::Url) -> Result<Option<ImportOutput>, AnyError> {
        let (plugin, _match) = if let Some(imp) = self.find_importer(url) {
            imp
        } else {
            return Ok(None);
        };

        let worker_lock = {
            self.state
                .read()
                .unwrap()
                .workers
                .get(&plugin.schema.name)
                .context(format!(
                    "No worker for plugin {} exists",
                    plugin.schema.name
                ))?
                .clone()
            // TODO: start new worker if none is present...
        };
        // TODO: timeout / multiple workers per plugin / concurrent workers
        let mut worker = worker_lock.lock().await;

        worker.send_import(url).await
    }

    async fn load_dir(
        plugin_dir: &PathBuf,
        config: &DenoConfig,
    ) -> Result<Vec<(PluginData, Worker)>, AnyError> {
        let mut plugins = Vec::new();

        for res in std::fs::read_dir(plugin_dir)? {
            let entry = res?;
            let path = entry.path();

            let is_typescript = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|x| x == "ts")
                .unwrap_or_default();

            if entry.file_type()?.is_file() && is_typescript {
                let code = std::fs::read_to_string(&path)?;

                let (worker, schema) = Self::boot_plugin_worker(&config.data_dir, &code).await?;

                let data = PluginData {
                    plugin_file: path,
                    schema,
                    code,
                };
                tracing::trace!(?data, "loaded deno plugin");
                plugins.push((data, worker));
            }
        }

        Ok(plugins)
    }

    async fn boot_plugin_worker(
        data_dir: &Path,
        code: &str,
    ) -> Result<(Worker, PluginSchema), AnyError> {
        // Ensure bridge.
        let script_dir = data_dir.join("scripts");
        if !script_dir.is_dir() {
            std::fs::create_dir_all(&script_dir)?;
        }
        let bridge_script_path = script_dir.join("__bridge.ts");
        std::fs::write(&bridge_script_path, BRIDGE_CODE)?;

        let filename = format!("{:x}.ts", sha2::Sha256::digest(code.as_bytes()));
        let worker_script_path = script_dir.join(filename);
        std::fs::write(&worker_script_path, code)?;

        let mut worker = Worker::boot(&bridge_script_path).await?;
        worker.send_ping().await?;
        let schema = worker.send_init(&worker_script_path).await?;
        worker.send_ping().await?;

        Ok((worker, schema))
    }

    // fn ensure_bridge_dir(data_dir: &Path) -> Result<PathBuf, anyhow::Error> {
    //     let p = data_dir.join("bridges");
    //     if !p.is_dir() {
    //         std::fs::create_dir_all(&p)?;
    //     }
    //     Ok(p)
    // }

    // fn run<T>(plugin_code: &str, config: &DenoConfig, command: PluginCommand) -> Result<T, AnyError>
    // where
    //     T: serde::de::DeserializeOwned,
    // {
    //     let cmd_code = match command {
    //         PluginCommand::Schema => "Promise.resolve(plugin.schema())".to_string(),
    //         PluginCommand::Import { url } => format!("plugin.import(\"{}\")", url),
    //     };

    //     let template = r#"
    // async function __semantic_run() {
    // let res;
    // try {
    //     // if (!('buildPlugin' in globalThis)) {
    //     //     throw new Error('Javascript module does not contain a buildPlugin() method!');
    //     // }
    //     const plugin = buildPlugin();
    //     const outputPromise = ${cmd_code};
    //     const output = await outputPromise;
    //     res = {Ok: output};
    // } catch (err) {
    //     res = {Err: err.toString()};
    // }

    // Deno.stderr.write(new TextEncoder().encode(JSON.stringify(res)));
    // }
    // __semantic_run();
    // "#;

    //     let run_code = template.replace("${cmd_code}", &cmd_code);

    //     let code = plugin_code.to_string() + "\n\n" + &run_code + "\n";

    //     let mut proc = Command::new("deno")
    //         .arg("run")
    //         // FIXME: use restrictive permissions
    //         .arg("-A")
    //         .stdin(std::process::Stdio::piped())
    //         .stdout(std::process::Stdio::piped())
    //         .stderr(std::process::Stdio::piped())
    //         // Read code from stdin
    //         .arg("-")
    //         .spawn()?;

    //     // Separate block to drop stdin once the code is sent.
    //     {
    //         let mut stdin = proc.stdin.take().unwrap();
    //         stdin.write_all(code.as_bytes())?;
    //     }

    //     let out = proc.wait_with_output()?;

    //     if !out.status.success() {
    //         let stdout = String::from_utf8_lossy(&out.stdout);
    //         let stderr = String::from_utf8_lossy(&out.stderr);
    //         bail!("Bridge failed: stdout: '{}' /// stderr: {}", stdout, stderr);
    //     }

    //     crate::util::json_from_slice::<Result<T, String>>(&out.stderr)?
    //         .map_err(|err| AnyError::msg(err))
    // }

    // fn run_schema(plugin_code: &str, config: &DenoConfig) -> Result<PluginSchema, AnyError> {
    //     Self::run(plugin_code, config, PluginCommand::Schema)
    // }

    // fn run_import(
    //     plugin_code: &str,
    //     config: &DenoConfig,
    //     url: String,
    // ) -> Result<Option<ImportOutput>, AnyError> {
    //     Self::run(plugin_code, config, PluginCommand::Import { url })
    // }
}

#[derive(serde::Serialize, Debug)]
enum PluginCommand {
    Init {
        plugin_path: String,
    },
    Ping,
    /// Import a url.
    Import {
        url: String,
    },
}

#[derive(serde::Deserialize, Debug)]
enum PluginReply {
    Init { schema: PluginSchema },
    Ping,
    Import { output: Option<ImportOutput> },
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_PLUGIN: &'static str = include_str!("./sample-plugin.ts");

    #[test]
    fn test_deno_worker() -> Result<(), AnyError> {
        let data_dir = std::env::temp_dir().join("semantic/deno/worker");
        if data_dir.is_dir() {
            std::fs::remove_dir_all(&data_dir)?;
        }
        std::fs::create_dir_all(&data_dir)?;

        let bridge = data_dir.join("bridge.ts");
        std::fs::write(&bridge, BRIDGE_CODE).unwrap();
        let plugin = data_dir.join("plugin.ts");
        std::fs::write(&plugin, SAMPLE_PLUGIN).unwrap();

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        rt.block_on(async {
            let mut worker = Worker::boot(&bridge).await.unwrap();

            worker.send_ping().await.unwrap();
            let schema = worker.send_init(&plugin).await.unwrap();
            dbg!(&schema);

            let url: url::Url = "http://test.com/abc".parse().unwrap();
            let output = worker
                .send_import(&url)
                .await
                .unwrap()
                .expect("epected a result");
            dbg!(&output);
            assert_eq!(output.items.len(), 1);
        });

        Ok(())
    }
}
