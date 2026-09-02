pub mod api;
pub mod fuse;
pub mod server;
pub mod shared;

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "semantic", about = "Semantic command-line interface")]
pub struct Args {
    #[command(subcommand)]
    pub command: SubCmd,
}

#[derive(Debug, Subcommand)]
pub enum SubCmd {
    /// Invoke the Semantic HTTP API.
    Api(api::Args),

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

    use super::{Args, SubCmd, api};

    #[test]
    fn parses_api_command_namespace() {
        let args = Args::try_parse_from([
            "semantic",
            "api",
            "get",
            "record-1",
            "--collection",
            "notes",
            "--scope",
            "research",
            "--pretty",
        ])
        .expect("parse api command");

        let SubCmd::Api(args) = args.command else {
            panic!("expected api command");
        };
        let api::SubCmd::Get(args) = args.command else {
            panic!("expected get command");
        };
        assert_eq!(args.id, "record-1");
        assert_eq!(args.collection.collection.as_deref(), Some("notes"));
        assert_eq!(args.client.scope.as_deref(), Some("research"));
        assert!(args.output.pretty);

        let mut command = Args::command();
        let help = command
            .find_subcommand_mut("api")
            .expect("api subcommand")
            .render_long_help()
            .to_string();
        for name in [
            "query", "get", "delete", "catalog", "package", "file", "apply", "upload",
        ] {
            assert!(help.contains(name), "missing api subcommand {name}");
        }
    }

    #[test]
    fn parses_all_first_wave_api_commands() {
        let commands: &[&[&str]] = &[
            &["semantic", "api", "query", "select 1", "--format", "sql"],
            &["semantic", "api", "get", "record-1"],
            &["semantic", "api", "delete", "record-1", "record-2", "--yes"],
            &["semantic", "api", "catalog"],
            &["semantic", "api", "package", "upsert", "package.json"],
            &["semantic", "api", "file", "analyze", "file-1"],
            &["semantic", "api", "apply", "batch.json"],
            &[
                "semantic",
                "api",
                "upload",
                "--recursive",
                "--entity",
                "entity.json",
                "files",
            ],
            &[
                "semantic",
                "api",
                "upload",
                "--tree",
                "--target-directory",
                "directory-1",
                "--replace",
                "--non-interactive",
                "files",
            ],
        ];

        for command in commands {
            Args::try_parse_from(*command).unwrap_or_else(|error| {
                panic!("failed to parse {command:?}: {error}");
            });
        }
    }

    #[test]
    fn parses_tree_upload_options() {
        let args = Args::try_parse_from([
            "semantic",
            "api",
            "upload",
            "--tree",
            "--target-dir",
            "directory-1",
            "--replace",
            "--non-interactive",
            "photos",
        ])
        .expect("parse tree upload");
        let SubCmd::Api(args) = args.command else {
            panic!("expected api command");
        };
        let api::SubCmd::Upload(args) = args.command else {
            panic!("expected upload command");
        };
        assert!(args.tree);
        assert!(args.replace);
        assert!(args.non_interactive);
        assert_eq!(args.target_directory.as_deref(), Some("directory-1"));
        assert_eq!(args.paths, vec![PathBuf::from("photos")]);

        assert!(
            Args::try_parse_from(["semantic", "api", "upload", "--replace", "photos"]).is_err()
        );
        assert!(
            Args::try_parse_from(["semantic", "api", "upload", "--non-interactive", "photos",])
                .is_err()
        );
        assert!(
            Args::try_parse_from([
                "semantic",
                "api",
                "upload",
                "--tree",
                "--recursive",
                "photos",
            ])
            .is_err()
        );
    }

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
        assert_eq!(args.client.rpc_url, "http://localhost:9000/api/v1/rpc");
        assert_eq!(args.client.scope.as_deref(), Some("research"));
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
