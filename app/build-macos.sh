#!/bin/sh
set -eu

app_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
cd "$app_dir"
npm run build
python3 "$app_dir/../tools/mon_third_party_notices.py"
python3 "$app_dir/../tools/mon_whisper_runtime.py"
python3 "$app_dir/../tools/mon_inference_runtime.py"
"$app_dir/../probes/macos-capture/build-bundle.sh" --configuration release --disable-sandbox
sh "$app_dir/../probes/speaker-embedding/build.sh" --out-dir "$app_dir/../probes/speaker-embedding/.build"
