use std::{path::PathBuf, process::Command};

const USAGE: &'static str = r#"
Semantic development CLI

Commands:
* git-pre-commit
* install-git-hooks
* build-ui
* build-ui
* watch-server
* watch-server
* watch-ui
* watch-ui
* build-server
* build-server
* build-appimage
* build-portable
* build
* install
* build-wasm-js
* help
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
        &["build-ui", "--dev"] => task_build_ui(false),
        &["watch-server"] => cmd_watch_server(true),
        &["watch-server", "--no-backend"] => cmd_watch_server(false),
        &["watch-ui"] => trunk_watch_ui(false),
        &["watch-ui", "--release"] => trunk_watch_ui(true),
        &["build-server"] => cmd_build_server(true),
        &["build-server", "--dev"] => cmd_build_server(false),
        &["build-appimage"] => cmd_build_appimage(),
        &["build-portable"] => cmd_build_portable(),
        &["build"] => cmd_build(),
        &["install"] => cmd_install(),
        &["build-wasm-js"] => gen_javascript(),
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

fn cmd_watch_server(default_backend: bool) -> Result<(), DynError> {
    // std::thread::spawn(|| {
    //     if let Err(err) = trunk_watch_ui() {
    //         eprintln!("UI WATCHER FAILED: {:?}", err);
    //         std::process::exit(1);
    //     }
    // });

    let data_path = root_path()?
        .join("data")
        .join("db.data")
        .to_str()
        .unwrap()
        .to_string();

    let mut cmd = Command::new("cargo");
    cmd.current_dir(root_path()?)
        .args(&["run", "--bin", "semantic", "--", "server"])
        // .arg("--deno-plugin-dir")
        // .arg(root_path()?.join("lib").join("contrib"))
        ;

    if default_backend {
        cmd.args(&[
            "--data-path",
            &data_path,
            "--key",
            "semantic",
            "--key-iterations",
            "1",
        ]);
    } else {
        cmd.arg("--no-backend");
    }

    if std::env::var("RUST_LOG").is_err() {
        cmd.env(
            "RUST_LOG", // "semantic=trace,semantic_core=trace,factordb=info",
            "trace",
        );
    }
    (&mut cmd).run()?;

    Ok(())
}

fn cmd_install_git_hooks() -> Result<(), DynError> {
    eprintln!("Installing git hooks...");
    devx_pre_commit::install_self_as_hook(&devx_pre_commit::locate_project_root()?)?;
    eprintln!("Git hooks installed!");
    Ok(())
}

fn cmd_build_server(release: bool) -> Result<(), DynError> {
    eprintln!("Building semantic...");
    let mut cmd = Command::new("cargo");
    cmd.args(&["build", "-p", "semantic"]);
    if release {
        cmd.arg("--release");
    }
    cmd.current_dir(root_path()?).run()?;
    eprintln!("Built!");
    Ok(())
}

fn cmd_build() -> Result<(), DynError> {
    eprintln!("Building ui...");
    task_build_ui(true)?;
    cmd_build_server(true)?;
    Ok(())
}

fn cmd_build_appimage() -> Result<(), DynError> {
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

fn cmd_build_portable() -> Result<(), DynError> {
    Command::new("docker")
        .args(dbg!(&[
            "run",
            "--rm",
            "-v",
            &format!("{}:/host", root_path()?.display()),
            "ubuntu",
            "bash",
            "-c",
            "/host/lib/docker/build.sh"
        ]))
        .run()?;
    Ok(())
}

fn cmd_install() -> Result<(), DynError> {
    eprintln!("Installing...");
    task_build_ui(true)?;
    Command::new("cargo")
        .env("SEMANTIC_UI_DIR", ui_dist_path()?)
        .args(&["install", "--path", "crates/semantic", "--features", "archive"])
        .current_dir(root_path()?)
        .run()?;
    eprintln!("Installed!");
    Ok(())
}

fn build_styles() -> Result<(), DynError> {
    eprintln!("Building styles...");
    Command::new("sassc")
        .arg(ui_path()?.join("assets").join("styles.scss"))
        .arg(ui_dist_path()?.join("styles.css"))
        .run()?;
    eprintln!("Styles built");
    Ok(())
}

fn task_build_ui(release: bool) -> Result<(), DynError> {
    eprintln!("Building ui...");
    let target_dir = ui_dist_path().unwrap();
    if !target_dir.is_dir() {
        std::fs::create_dir_all(&target_dir).unwrap();
    }

    let rustflags = vec!["--cfg=web_sys_unstable_apis" /*, "-Cdebuginfo=0"*/];

    let mut cmd = Command::new("wasm-pack");
    cmd.env("RUSTFLAGS", rustflags.join(" "))
        .current_dir(ui_path()?)
        .env("CARGO_TARGET_DIR", wasm_target_path()?)
        .args(&["build", "--target", "web"])
        .arg("--out-dir")
        .arg(&target_dir)
        .arg(ui_path()?);
    if !release {
        cmd.arg("--dev");
    }
    eprintln!("Building ui crate...\n{:?}", cmd);
    (&mut cmd).run()?;

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

fn trunk_watch_ui(release: bool) -> Result<(), DynError> {
    let cmd = if release {
        "cargo xtask build-ui"
    } else {
        "cargo xtask build-ui --dev"
    };
    Command::new("cargo")
        .current_dir(root_path()?)
        .args(&[
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

fn gen_javascript() -> Result<(), DynError> {
    // let mut gen = witx_bindgen_gen_spidermonkey::SpiderMonkeyWasm::new("foo.js", "");
    // gen.import_spidermonkey(true);
    Ok(())
}

fn root_path() -> Result<PathBuf, DynError> {
    if let Ok(manifest) = std::env::var("CARGO_MANIFEST_DIR") {
        let path = PathBuf::from(manifest);
        path.parent()
            .and_then(|x| x.parent())
            .map(|x| x.to_path_buf())
            .ok_or_else(|| "Could not find project root".into())
    } else {
        Ok(std::env::current_dir()?)
    }
}

fn ui_dist_path() -> Result<PathBuf, DynError> {
    root_path().map(|p| p.join("target").join("ui"))
}

fn wasm_target_path() -> Result<PathBuf, DynError> {
    root_path().map(|p| p.join("target/wasm"))
}

fn ui_path() -> Result<PathBuf, DynError> {
    root_path().map(|p| p.join("crates").join("ui"))
}

trait CommandExt {
    fn run(&mut self) -> Result<(), DynError>;
}

impl CommandExt for &mut Command {
    fn run(&mut self) -> Result<(), DynError> {
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
