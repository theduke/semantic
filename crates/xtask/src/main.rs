use std::{path::PathBuf, process::Command};

const USAGE: &'static str = r#"
Semantic development CLI

Commands:
* build-ui
  Build the UI in release mode.
* watch
  Run a development server and watch/auto-rebuild the UI.
* install
  Install the `semantic` binary locally via `cargo install`.
* install-git-hooks
  Install a pre-commit hook that runst `rustfmt` on Git staged changes only.
"#;

type DynError = Box<dyn std::error::Error>;

fn main() -> Result<(), DynError> {
    // This is just a simple development CLI, so just do some manual argument
    // parsing instead of pulling in a dependency like clap/structopt.
    let args: Vec<_> = std::env::args().skip(1).collect();
    let args_str: Vec<_> = args.iter().map(|x| x.as_str()).collect();

    match args_str.as_slice() {
        &["git-pre-commit"] => cmd_git_pre_commit(),
        &["install-git-hooks"] => cmd_install_git_hooks(),
        &["build-ui"] => task_build_ui(true),
        &["watch"] => cmd_watch(),
        &["install"] => cmd_install(),
        &["help"] => {
            eprintln!("{}", USAGE);
            Ok(())
        }
        &[first, ..] => {
            eprintln!("Error: Unknown command: {}", first);
            eprintln!("{}", USAGE);
            std::process::exit(1);
        }
        &[] => {
            eprintln!("Error: No command specified");
            eprintln!("{}", USAGE);
            std::process::exit(1);
        }
    }
}

fn cmd_git_pre_commit() -> Result<(), DynError> {
    let mut ctx =
        devx_pre_commit::PreCommitContext::from_git_diff(devx_pre_commit::locate_project_root()?)?;

    // Optionally filter out the files you don't want to format
    ctx.retain_staged_files(|_path| true);

    // Run `cargo fmt` against the crates with staged rust source files
    ctx.rustfmt()?;

    // Stage all the changes potenitally introduced by rustfmt
    // It is super-important to call this method at the end of the hook
    ctx.stage_new_changes()?;
    Ok(())
}

fn cmd_watch() -> Result<(), DynError> {
    std::thread::spawn(|| {
        if let Err(err) = trunk_watch_ui() {
            eprintln!("UI WATCHER FAILED: {:?}", err);
            std::process::exit(1);
        }
    });

    let data_path = root_path()?
        .join("data")
        .join("db.data")
        .to_str()
        .unwrap()
        .to_string();

    let mut cmd = Command::new("cargo");
    cmd.current_dir(root_path()?).args(&[
        "run",
        "--bin",
        "semantic",
        "--",
        "server",
        "--data-path",
        &data_path,
        "--key",
        "testkey",
    ]);

    if std::env::var("RUST_LOG").is_err() {
        cmd.env(
            "RUST_LOG",
            "semantic=trace,semantic_core=trace,factordb=info",
        );
    }
    // TODO: check if lld is available
    if true {
        cmd.env(
            "RUSTFLAGS",
            "-C link-arg=-fuse-ld=lld --cfg=web_sys_unstable_apis",
        );
    }
    (&mut cmd).spawn_success()?;

    Ok(())
}

fn cmd_install_git_hooks() -> Result<(), DynError> {
    eprintln!("Installing git hooks...");
    devx_pre_commit::install_self_as_hook(&devx_pre_commit::locate_project_root()?)?;
    eprintln!("Git hooks installed!");
    Ok(())
}

fn cmd_install() -> Result<(), DynError> {
    eprintln!("Installing...");
    task_build_ui(true)?;
    Command::new("cargo")
        .args(&["install", "--path", "crates/semantic"])
        .current_dir(root_path()?)
        .spawn_success()?;
    eprintln!("Installed!");
    Ok(())
}

fn task_build_ui(release: bool) -> Result<(), DynError> {
    eprintln!("Building UI (release: {})...", release);
    let mut cmd = build_trunk_command("build")?;
    if release {
        cmd.arg("--release");
    }
    (&mut cmd).spawn_success()?;
    eprintln!("UI built");
    Ok(())
}

fn trunk_watch_ui() -> Result<(), DynError> {
    eprintln!("Watching UI...");
    let mut cmd = build_trunk_command("watch")?;
    (&mut cmd).spawn_success()?;
    Ok(())
}

fn build_trunk_command(action: &str) -> Result<Command, DynError> {
    let wasm_target = root_path()?.join("target/wasm");
    let mut cmd = Command::new("trunk");
    cmd.current_dir(ui_path()?)
        .arg(action)
        .args(&["--public-url", "/assets"])
        .arg("--dist")
        .arg(ui_dist_path()?)
        .env("RUSTFLAGS", "--cfg=web_sys_unstable_apis")
        .env("CARGO_TARGET_DIR", wasm_target);
    Ok(cmd)
}

fn root_path() -> Result<PathBuf, DynError> {
    let manifest = std::env::var("CARGO_MANIFEST_DIR")?;
    let path = PathBuf::from(manifest);
    path.parent()
        .and_then(|x| x.parent())
        .map(|x| x.to_path_buf())
        .ok_or_else(|| "Could not find project root".into())
}

fn ui_dist_path() -> Result<PathBuf, DynError> {
    root_path().map(|p| p.join("target").join("ui"))
}

fn ui_path() -> Result<PathBuf, DynError> {
    root_path().map(|p| p.join("crates").join("ui"))
}

trait CommandExt {
    fn spawn_success(&mut self) -> Result<(), DynError>;
}

impl CommandExt for &mut Command {
    fn spawn_success(&mut self) -> Result<(), DynError> {
        self.spawn()?.wait()?.ensure_success()?;
        Ok(())
    }
}

trait SuccessExt: Sized {
    fn ensure_success(self) -> Result<Self, DynError>;
}

impl SuccessExt for std::process::Output {
    fn ensure_success(self) -> Result<Self, DynError> {
        if !self.status.success() {
            Err(format!("Command failed with exit code '{}'", self.status).into())
        } else {
            Ok(self)
        }
    }
}

impl SuccessExt for std::process::ExitStatus {
    fn ensure_success(self) -> Result<Self, DynError> {
        if !self.success() {
            Err(format!("Command failed with exit code '{}'", self).into())
        } else {
            Ok(self)
        }
    }
}
