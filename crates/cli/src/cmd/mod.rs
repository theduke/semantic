pub mod fuse;

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "semantic", about = "Semantic command-line interface")]
pub struct Args {
    #[command(subcommand)]
    pub command: SubCmd,
}

#[derive(Debug, Subcommand)]
pub enum SubCmd {
    /// Mount a semantic database as a filesystem.
    Fuse(fuse::Args),
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use clap::{CommandFactory, Parser};
    use semantic_fuse::EntityFormat;

    use super::{Args, SubCmd};

    #[test]
    fn parses_fuse_command_and_renders_help() {
        let args = Args::try_parse_from([
            "semantic",
            "fuse",
            "/mnt/semantic",
            "--rpc-url",
            "http://localhost:9000/api/v1/rpc",
            "--scope",
            "research",
            "--format",
            "yaml",
            "--allow-other",
            "--read-only",
        ])
        .expect("parse fuse command");

        let SubCmd::Fuse(args) = args.command;
        assert_eq!(args.mountpoint, PathBuf::from("/mnt/semantic"));
        assert_eq!(args.rpc_url, "http://localhost:9000/api/v1/rpc");
        assert_eq!(args.scope.as_deref(), Some("research"));
        assert_eq!(args.format, EntityFormat::Yaml);
        assert!(args.allow_other);
        assert!(args.read_only);

        let mut command = Args::command();
        let help = command
            .find_subcommand_mut("fuse")
            .expect("fuse subcommand")
            .render_long_help()
            .to_string();
        assert!(help.contains("<MOUNTPOINT>"));
        assert!(help.contains("--rpc-url"));
        assert!(help.contains("--scope"));
        assert!(help.contains("--format"));
        assert!(help.contains("--allow-other"));
        assert!(help.contains("--read-only"));
    }
}
