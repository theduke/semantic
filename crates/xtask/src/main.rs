use std::collections::VecDeque;

const USAGE: &'static str = r#"
Semantic development CLI

Commands:
* git-pre-commit
  Install a pre-commit hook that runst `rustfmt` on Git staged changes only
"#;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // This is just a simple development CLI, so just do some manual argument
    // parsing instead of pulling in a dependency like clap/structopt.

    let args: Vec<_> = std::env::args().skip(1).collect();
    let mut args_str: VecDeque<_> = args.iter().map(|x| x.as_str()).collect();

    match args_str.pop_front() {
        Some("git-pre-commit") => {
            let mut ctx = devx_pre_commit::PreCommitContext::from_git_diff(
                devx_pre_commit::locate_project_root()?,
            )?;

            // Optionally filter out the files you don't want to format
            ctx.retain_staged_files(|_path| true);

            // Run `cargo fmt` against the crates with staged rust source files
            ctx.rustfmt()?;

            // Stage all the changes potenitally introduced by rustfmt
            // It is super-important to call this method at the end of the hook
            ctx.stage_new_changes()?;
            Ok(())
        }
        Some("install-git-hooks") => {
            eprintln!("Installing git hooks...");
            devx_pre_commit::install_self_as_hook(&devx_pre_commit::locate_project_root()?)?;
            eprintln!("Git hooks installed!");
            Ok(())
        }
        Some("help") => {
            eprintln!("{}", USAGE);
            Ok(())
        }
        Some(other) => {
            eprintln!("Error: Unknown command: {}", other);
            eprintln!("{}", USAGE);
            std::process::exit(1);
        }
        None => {
            eprintln!("Error: No command specified");
            eprintln!("{}", USAGE);
            std::process::exit(1);
        }
    }
}
