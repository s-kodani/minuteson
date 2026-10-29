#!/bin/sh
set -eu
probe_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_dir=$(CDPATH= cd -- "$probe_dir/../.." && pwd)
cargo build --manifest-path "$repo_dir/Cargo.toml" -p mon-capture-ffi --release --locked --target-dir "$probe_dir/.build/capture-core"
