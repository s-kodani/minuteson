#!/bin/sh
set -eu

app_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
cd "$app_dir"
npm run build
"$app_dir/../probes/macos-capture/build-bundle.sh" --configuration release --disable-sandbox
sh "$app_dir/../probes/speaker-embedding/build.sh" --out-dir "$app_dir/../probes/speaker-embedding/.build"
