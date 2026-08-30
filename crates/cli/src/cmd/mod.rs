pub mod fuse;
pub mod server;

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

    /// Run the semantic database server.
    Server(server::Args),
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

        let SubCmd::Fuse(args) = args.command else {
            panic!("expected fuse command");
        };
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

    #[test]
    fn parses_server_command_and_renders_help() {
        let args = Args::try_parse_from([
            "semantic",
            "server",
            "--db",
            "/var/lib/semantic/db",
            "--blob-uri",
            "fs:///var/lib/semantic/blob",
            "--bind",
            "0.0.0.0:9000",
            "--data-dir",
            "/var/lib/semantic",
            "--temp-dir",
            "/tmp/semantic",
            "--auto-analyze-media",
            "--interface",
            "127.0.0.1",
            "--port",
            "8888",
        ])
        .expect("parse server command");

        let SubCmd::Server(args) = args.command else {
            panic!("expected server command");
        };
        assert_eq!(args.db, Some(PathBuf::from("/var/lib/semantic/db")));
        assert_eq!(
            args.blob_uri.as_deref(),
            Some("fs:///var/lib/semantic/blob")
        );
        assert_eq!(args.bind.as_deref(), Some("0.0.0.0:9000"));
        assert_eq!(args.data_dir, Some(PathBuf::from("/var/lib/semantic")));
        assert_eq!(args.temp_dir, Some(PathBuf::from("/tmp/semantic")));
        assert!(args.auto_analyze_media);
        assert_eq!(args.interface.as_deref(), Some("127.0.0.1"));
        assert_eq!(args.port, Some(8888));

        let mut command = Args::command();
        let help = command
            .find_subcommand_mut("server")
            .expect("server subcommand")
            .render_long_help()
            .to_string();
        assert!(help.contains("--db"));
        assert!(help.contains("--blob-uri"));
        assert!(help.contains("--bind"));
        assert!(help.contains("--data-dir"));
        assert!(help.contains("--temp-dir"));
        assert!(help.contains("--auto-analyze-media"));
        assert!(help.contains("--interface"));
        assert!(help.contains("--port"));
    }
}
