#! /usr/bin/env python3

import subprocess
import argparse
import os
import pathlib
from pprint import pprint

SCRIPT_PATH = os.path.realpath(__file__)
ROOT_DIR = os.path.dirname(SCRIPT_PATH)
WASM_TARGET = os.path.join(ROOT_DIR, "target/wasm")
UI_TARGET = os.path.join(ROOT_DIR, "target/ui")

parser = argparse.ArgumentParser(description = 'Semantic build helper')
subparsers = parser.add_subparsers(dest='command')

cmd_build_release = subparsers.add_parser('build-release')
cmd_build = subparsers.add_parser('build')
cmd_ui = subparsers.add_parser('ui')
cmd_watch = subparsers.add_parser('watch')
cmd_run = subparsers.add_parser('run')

args = parser.parse_args()

cmd = args.command

def build_ui(release: bool):
    args = [
        'trunk',
        'build',
        '--public-url',
        '/assets',
        '--dist',
        UI_TARGET,
    ];
    if release:
        args.append('--release')
    args.append(os.path.join(ROOT_DIR, "crates", "ui", "index.html"))
    subprocess.run(args, check=True, env = {**os.environ, 'RUSTFLAGS': '--cfg=web_sys_unstable_apis', 'CARGO_TARGET_DIR': WASM_TARGET})

def serve_ui():
    args = [
        'trunk',
        # '--config',
        # os.path.join(ROOT_DIR, 'crates/ui/Trunk.toml'),
        'serve',
        '--public-url',
        '/assets',
        '--dist',
        UI_TARGET,
    ];
    args.append(os.path.join(ROOT_DIR, "crates/ui/index.html"))
    subprocess.run(
        check=True, 
        cwd=os.path.join(ROOT_DIR, "crates", "ui"),
        args=args,
        env = {**os.environ, 'RUSTFLAGS': '--cfg=web_sys_unstable_apis', 'CARGO_TARGET_DIR': WASM_TARGET},
    )


def watch_ui():
    args = [
        'trunk',
        # '--config',
        # os.path.join(ROOT_DIR, 'crates/ui/Trunk.toml'),
        'watch',
        '--public-url',
        '/assets',
        '--dist',
        UI_TARGET,
    ];
    args.append(os.path.join(ROOT_DIR, "crates/ui/index.html"))

    pprint(args)

    subprocess.run(
        check=True, 
        cwd=os.path.join(ROOT_DIR, "crates", "ui"),
        args=args,
        env = {**os.environ, 'RUSTFLAGS': '--cfg=web_sys_unstable_apis', 'CARGO_TARGET_DIR': WASM_TARGET},
    )

def run():
    subprocess.run(
        check=True,
        cwd=ROOT_DIR,
        args=[
            'cargo',
            'run',
            '--bin',
            'semantic',
            '--',
            'server',
            '--data-path=' + os.path.join(ROOT_DIR, 'data/db.data'),
        ],
        env={
            **os.environ,
            'RUSTFLAGS': "-C link-arg=-fuse-ld=lld --cfg=web_sys_unstable_apis",
            'RUST_LOG': 'semantic=trace,semantic_core=trace,factordb=info',
        },
    )

if cmd == 'build-release':
    build_ui(True)
elif cmd == 'build':
    build_ui(False)
elif cmd == 'ui':
    serve_ui()
elif cmd == 'watch':
    watch_ui()
elif cmd == 'run':
    run()
else:
    raise "Unknown command " + cmd
