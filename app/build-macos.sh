#!/bin/sh
set -eu

app_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
cd "$app_dir"
npm run build
"$app_dir/../probes/macos-capture/build-bundle.sh" --disable-sandbox
