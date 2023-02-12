use std::{path::PathBuf, process::Command};

use clap::Parser;

#[derive(clap::Parser)]
struct Args {
    #[command(subcommand)]
    cmd: Subcommand,
}

#[derive(clap::Subcommand)]
enum Subcommand {
    GitPreCommit,
    GitInstallHooks,
    BuildUi {
        #[arg(long)]
        release: bool,
    },
    WatchServer {
        #[arg(long)]
        no_backend: bool,
    },
    WatchUi {
        #[clap(long)]
        release: bool,
    },
    BuildCli {
        #[arg(long)]
        release: bool,
    },
    BuildAppimage,
    BuildPortable,
    Build,
    Install,
    BuildTypescript,
}

fn main() -> Result<(), anyhow::Error> {
    let args = Args::parse();
    match args.cmd {
        Subcommand::GitPreCommit => cmd_git_pre_commit(),
        Subcommand::GitInstallHooks => cmd_install_git_hooks(),
        Subcommand::BuildUi { release } => task_build_ui(release),
        Subcommand::WatchServer { no_backend } => cmd_watch_server(!no_backend),
        Subcommand::WatchUi { release } => cmd_watch_ui(release),
        Subcommand::BuildCli { release } => cmd_build_cli(release),
        Subcommand::BuildAppimage => cmd_build_appimage(),
        Subcommand::BuildPortable => cmd_build_portable(),
        Subcommand::Build => cmd_build(),
        Subcommand::Install => cmd_install(),
        Subcommand::BuildTypescript => cmd_build_typescript(),
    }
}

fn cmd_git_pre_commit() -> Result<(), anyhow::Error> {
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

fn cmd_watch_server(default_backend: bool) -> Result<(), anyhow::Error> {
    // std::thread::spawn(|| {
    //     if let Err(err) = trunk_watch_ui() {
    //         eprintln!("UI WATCHER FAILED: {:?}", err);
    //         std::process::exit(1);
    //     }
    // });

    let data_dir = root_path()?.join("data");

    let db_path = data_dir.join("db.data").to_str().unwrap().to_string();

    let mut cmd = Command::new("cargo");
    cmd.current_dir(root_path()?)
        .args(["run", "--bin", "semantic", "--", "serve"])
        // .arg("--deno-plugin-dir")
        // .arg(root_path()?.join("lib").join("contrib"))
        ;

    if default_backend {
        cmd.args([
            "--data-path",
            &db_path,
            "--key",
            "semantic",
            "--key-iterations",
            "1",
        ]);
    } else {
        cmd.arg("--no-backend");
    }

    cmd.arg("--tmp-dir");
    cmd.arg(data_dir.join("tmp"));

    if std::env::var("RUST_LOG").is_err() {
        cmd.env(
            "RUST_LOG",
            "semantic=trace,semantic_core=trace,factordb=info",
        );
    }
    cmd.run()?;

    Ok(())
}

fn cmd_install_git_hooks() -> Result<(), anyhow::Error> {
    eprintln!("Installing git hooks...");
    devx_pre_commit::install_self_as_hook(&devx_pre_commit::locate_project_root()?)?;
    eprintln!("Git hooks installed!");
    Ok(())
}

fn cmd_build_cli(release: bool) -> Result<(), anyhow::Error> {
    eprintln!("Building CLI binary...");
    let mut cmd = Command::new("cargo");
    cmd.current_dir(root_path()?.join("crates/cli"));
    cmd.args(["build"]);
    if release {
        cmd.arg("--release");
    }

    cmd.run()?;

    eprintln!("Built!");
    Ok(())
}

fn build_logfs(release: bool) -> Result<(), anyhow::Error> {
    eprintln!("Building logfs binary...");
    let mut cmd = Command::new("cargo");
    cmd.current_dir(root_path()?.join("crates/logfs"));
    let mut cmd = Command::new("cargo");
    cmd.args(["build"]);
    if release {
        cmd.arg("--release");
    }

    cmd.run()?;

    Ok(())
}

fn cmd_build() -> Result<(), anyhow::Error> {
    eprintln!("Building ui...");
    task_build_ui(true)?;
    cmd_build_cli(true)?;
    build_logfs(true)?;
    Ok(())
}

fn cmd_build_appimage() -> Result<(), anyhow::Error> {
    cmd_build()?;

    let build_dir = root_path()?.join("target").join("appimage");
    let src = build_dir.join("build");
    if src.is_dir() {
        std::fs::remove_dir_all(&src)?;
    }
    let asset_dir = root_path()?.join("lib/appimage");
    std::fs::create_dir_all(&src)?;

    std::fs::copy(
        root_path()?.join("target/release/semantic"),
        src.join("semantic"),
    )
    .unwrap();
    std::fs::copy(asset_dir.join("appicon.png"), src.join("appicon.png")).unwrap();
    std::fs::copy(
        asset_dir.join("semantic.desktop"),
        src.join("semantic.desktop"),
    )
    .unwrap();

    Command::new("appimagetool")
        .arg(&src)
        .arg(build_dir.join("semantic.AppImage"))
        .run()?;
    Ok(())
}

fn cmd_build_portable() -> Result<(), anyhow::Error> {
    Command::new("docker")
        .args([
            "run",
            "--rm",
            "-v",
            &format!("{}:/host", root_path()?.display()),
            "debian:bullseye",
            "bash",
            "-c",
            "/host/lib/docker/build.sh",
        ])
        .run()?;
    Ok(())
}

fn cmd_install() -> Result<(), anyhow::Error> {
    eprintln!("Installing...");
    task_build_ui(true)?;
    Command::new("cargo")
        .env("SEMANTIC_UI_DIR", ui_dist_path()?)
        .args(["install", "--path", "crates/cli"])
        .current_dir(root_path()?)
        .run()?;
    eprintln!("Installed!");
    Ok(())
}

fn build_styles() -> Result<(), anyhow::Error> {
    eprintln!("Building styles...");
    Command::new("sassc")
        .arg(ui_path()?.join("assets").join("styles.scss"))
        .arg(ui_dist_path()?.join("styles.css"))
        .run()?;
    eprintln!("Styles built");
    Ok(())
}

fn build_ui_v2() -> Result<(), anyhow::Error> {
    eprintln!("Building UI v2...");

    let js_path = root_path()?.join("js").join("ui");

    Command::new("yarn")
        .args(["build"])
        .current_dir(&js_path)
        .run()?;

    let build_path = js_path.join("dist");
    let target_path = root_path()?.join("target/ui2");

    std::fs::create_dir_all(target_path.parent().unwrap())?;

    if target_path.exists() {
        std::fs::remove_dir_all(&target_path)?;
    }
    std::fs::rename(&build_path, &target_path)?;

    eprintln!("UI v2 built");
    Ok(())
}

fn task_build_ui(release: bool) -> Result<(), anyhow::Error> {
    build_ui_v2()?;

    eprintln!("Building ui...");
    let target_dir = ui_dist_path()?;
    if !target_dir.is_dir() {
        std::fs::create_dir_all(&target_dir).unwrap();
    }

    let wasm_target = wasm_target_path()?;
    if !wasm_target.is_dir() {
        std::fs::create_dir_all(&wasm_target)?;
    }

    let rustflags = vec!["--cfg=web_sys_unstable_apis" /*, "-Cdebuginfo=0"*/];

    let mut cmd = Command::new("wasm-pack");
    cmd.env("RUSTFLAGS", rustflags.join(" "))
        .current_dir(ui_path()?)
        .env("CARGO_TARGET_DIR", wasm_target)
        .args(["build", "--target", "web"])
        .arg("--out-dir")
        .arg(&target_dir)
        .arg(ui_path()?);
    if !release {
        cmd.arg("--dev");
    }
    eprintln!("Building ui crate...\n{:?}", cmd);
    cmd.run()?;

    eprintln!("Building styles...");
    build_styles()?;

    eprintln!("Copying index.html...");
    std::fs::copy(
        ui_path()?.join("index.html"),
        ui_dist_path()?.join("index.html"),
    )?;

    eprintln!("Copying webfonts...");
    let font_dir = ui_dist_path()?.join("webfonts");
    std::fs::create_dir_all(&font_dir)?;
    for res in std::fs::read_dir(ui_path()?.join("assets/fontawesome-free-5.15.4-web/webfonts"))? {
        let entry = res?;
        let target = font_dir.join(entry.file_name());
        std::fs::copy(entry.path(), target)?;
    }

    eprintln!("UI built");

    Ok(())
}

fn cmd_watch_ui(release: bool) -> Result<(), anyhow::Error> {
    let cmd = if release {
        "cargo xtask build-ui"
    } else {
        "cargo xtask build-ui --dev"
    };
    Command::new("cargo")
        .current_dir(root_path()?)
        .args([
            "watch",
            "--ignore",
            "crates/semantic/*",
            "--ignore",
            "crates/logfs/*",
            "--ignore",
            "crates/xtask/*",
            "--ignore",
            "target/*",
            "--ignore",
            "data/*",
            "--ignore",
            "docs/*",
            "--ignore",
            "lib/*",
            "--shell",
            cmd,
        ])
        .run()
}

fn cmd_build_typescript() -> Result<(), anyhow::Error> {
    eprintln!("Generating typescript types...");

    let res = Command::new("cargo")
        .args([
            "run",
            "-p",
            "semantic_core",
            "--bin",
            "generate-schema",
            "--features",
            "schema",
        ])
        .output()?;
    if !res.status.success() {
        eprintln!("{}", String::from_utf8_lossy(&res.stdout));
        eprintln!("{}", String::from_utf8_lossy(&res.stderr));
        anyhow::bail!("schema generation failed!");
    }
    let schema = std::str::from_utf8(&res.stdout)?;

    let out_dir = root_path()?.join("js").join("semantic").join("src");

    let core_path = out_dir.join("core.ts");
    std::fs::write(&core_path, schema)?;
    eprintln!("Wrote core types to {}", core_path.display());

    eprintln!("Generating entity type schemas from database...");

    let res = Command::new("cargo")
        .args(["run", "-p", "semantic_cli", "--", "generate-typescript"])
        .output()?;
    if !res.status.success() {
        let err = std::str::from_utf8(&res.stderr)?;
        eprintln!("Cargo failed: \n{}", err);
        anyhow::bail!("failed to run 'semantic generate-typescript'");
    }

    let schema = std::str::from_utf8(&res.stdout)?;

    let schema_path = out_dir.join("schema.ts");
    std::fs::write(&schema_path, schema)?;
    eprintln!("wrote schema to {}", schema_path.display());

    Ok(())
}

fn root_path() -> Result<PathBuf, anyhow::Error> {
    if let Ok(manifest) = std::env::var("CARGO_MANIFEST_DIR") {
        let path = PathBuf::from(manifest);
        path.parent()
            .and_then(|x| x.parent())
            .map(|x| x.to_path_buf())
            .ok_or_else(|| anyhow::anyhow!("Could not find project root"))
    } else {
        Ok(std::env::current_dir()?)
    }
}

fn cargo_target_dir() -> Result<PathBuf, anyhow::Error> {
    if let Ok(p) = std::env::var("CARGO_TARGET_DIR") {
        Ok(PathBuf::from(p))
    } else {
        root_path().map(|p| p.join("target"))
    }
}

fn ui_dist_path() -> Result<PathBuf, anyhow::Error> {
    root_path().map(|p| p.join("target").join("ui"))
}

fn wasm_target_path() -> Result<PathBuf, anyhow::Error> {
    cargo_target_dir().map(|p| p.join("wasm"))
}

fn ui_path() -> Result<PathBuf, anyhow::Error> {
    root_path().map(|p| p.join("crates").join("ui"))
}

trait CommandExt {
    fn run(&mut self) -> Result<(), anyhow::Error>;
}

impl CommandExt for &mut Command {
    fn run(&mut self) -> Result<(), anyhow::Error> {
        self.spawn()?.wait()?.ensure_success()?;
        Ok(())
    }
}

impl CommandExt for Command {
    fn run(&mut self) -> Result<(), anyhow::Error> {
        self.spawn()?.wait()?.ensure_success()?;
        Ok(())
    }
}

trait SuccessExt: Sized {
    fn ensure_success(self) -> Result<Self, anyhow::Error>;
}

impl SuccessExt for std::process::Output {
    fn ensure_success(self) -> Result<Self, anyhow::Error> {
        if !self.status.success() {
            Err(anyhow::anyhow!(
                "Command failed with exit code '{}'",
                self.status
            ))
        } else {
            Ok(self)
        }
    }
}

impl SuccessExt for std::process::ExitStatus {
    fn ensure_success(self) -> Result<Self, anyhow::Error> {
        if !self.success() {
            Err(anyhow::anyhow!("Command failed with exit code '{}'", self))
        } else {
            Ok(self)
        }
    }
}
