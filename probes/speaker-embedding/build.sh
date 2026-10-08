#!/bin/sh
set -eu

if [ "$#" -ne 2 ] || [ "$1" != --out-dir ]; then
  printf '%s\n' 'usage: sh build.sh --out-dir /absolute/output/directory' >&2
  exit 1
fi
case "$2" in /*) ;; *) printf '%s\n' 'output directory must be absolute' >&2; exit 1 ;; esac
if [ "$(uname -s)" != Darwin ]; then
  printf '%s\n' 'the headless embedding helper currently supports macOS only' >&2
  exit 1
fi
mon_embedding_source=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
mkdir -p "$2"
mon_embedding_output=$(CDPATH= cd -- "$2" && pwd)
rustc --edition=2021 -C opt-level=2 -C debug-assertions=yes -C overflow-checks=yes \
  "$mon_embedding_source/main.rs" -o "$mon_embedding_output/mon-speaker-embedding"
codesign --force --sign - "$mon_embedding_output/mon-speaker-embedding"
codesign --verify --strict "$mon_embedding_output/mon-speaker-embedding"
printf '%s\n' "$mon_embedding_output/mon-speaker-embedding"
