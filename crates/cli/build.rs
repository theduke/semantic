//! Embeds the git commit hash into the binary as `SEMANTIC_GIT_COMMIT`.
//!
//! The `SEMANTIC_GIT_COMMIT` environment variable takes precedence, which
//! allows builds without a `.git` directory (e.g. Nix) to provide the hash.

use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=SEMANTIC_GIT_COMMIT");

    let commit = std::env::var("SEMANTIC_GIT_COMMIT")
        .ok()
        .filter(|c| !c.is_empty())
        .or_else(git_commit)
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=SEMANTIC_GIT_COMMIT={commit}");
}

fn git_commit() -> Option<String> {
    // Resolve paths through Git so linked worktrees and packed refs work too.
    for name in ["HEAD", "refs", "packed-refs"] {
        if let Some(path) = git(&["rev-parse", "--path-format=absolute", "--git-path", name]) {
            // A missing watched path would make Cargo rebuild on every invocation.
            if std::path::Path::new(&path).exists() {
                println!("cargo:rerun-if-changed={path}");
            }
        }
    }
    git(&["rev-parse", "--short=12", "HEAD"])
}

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8(out.stdout).ok()?.trim().to_string())
}
