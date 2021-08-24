#! /usr/bin/env python3

import subprocess
import argparse
import os
import pathlib
from pprint import pprint

SCRIPT_PATH = os.path.realpath(__file__)
ROOT_DIR = os.path.dirname(SCRIPT_PATH)
WASM_TARGET = os.path.join(ROOT_DIR, "target_wasm")
UI_TARGET = os.path.join(WASM_TARGET, "ui")

parser = argparse.ArgumentParser(description = 'Semantic build helper')
subparsers = parser.add_subparsers(dest='command')

cmd_build_release = subparsers.add_parser('build-release')

args = parser.parse_args()

cmd = args.command

if cmd == 'build-release':
    subprocess.run([
        'trunk',
        'build',
        '--release',
        '--public-url',
        '/assets',
        '--dist',
        UI_TARGET,
        os.path.join(ROOT_DIR, "semantic_ui", "index.html"),
        ], 
        check=True, 
        env = {**os.environ, 'RUSTFLAGS': '', 'CARGO_TARGET_DIR': WASM_TARGET}
    )

    # for relative_path in os.listdir(UI_TARGET):
    #     path = os.path.join(UI_TARGET, relative_path)
    #     pp = pathlib.Path(path)
    #     ext = pp.suffix

    #     has_hash = pp.stem.find('-') != -1
    #     if has_hash:
    #         clean_name = pp.stem.split('-')[0]
    #         new_path = os.path.join(pp.parent, clean_name + pp.suffix)
    #         os.rename(path, new_path)
else:
    raise "Unknown command " + cmd
