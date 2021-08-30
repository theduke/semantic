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
cmd_build_release = subparsers.add_parser('build')

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
    args.append(os.path.join(ROOT_DIR, "crates/ui/index.html"))
    subprocess.run(args, check=True, env = {**os.environ, 'RUSTFLAGS': '', 'CARGO_TARGET_DIR': WASM_TARGET})

if cmd == 'build-release':
    build_ui(True)
elif cmd == 'build':
    build_ui(False)
else:
    raise "Unknown command " + cmd
