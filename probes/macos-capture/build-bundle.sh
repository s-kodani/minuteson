#!/bin/sh
set -eu

probe_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
cd "$probe_dir"
SWIFTPM_MODULECACHE_OVERRIDE=${SWIFTPM_MODULECACHE_OVERRIDE:-$probe_dir/.build/module-cache/swift}
CLANG_MODULE_CACHE_PATH=${CLANG_MODULE_CACHE_PATH:-$probe_dir/.build/module-cache/clang}
export SWIFTPM_MODULECACHE_OVERRIDE CLANG_MODULE_CACHE_PATH
mkdir -p "$SWIFTPM_MODULECACHE_OVERRIDE" "$CLANG_MODULE_CACHE_PATH"
swift build "$@"
binary_dir=$(swift build --show-bin-path "$@")
bundle_dir="$probe_dir/.build/MONCaptureProbe.app"
mkdir -p "$bundle_dir/Contents/MacOS"
install -m 755 "$binary_dir/mon-macos-capture" "$bundle_dir/Contents/MacOS/mon-macos-capture"
install -m 644 "$probe_dir/Info.plist" "$bundle_dir/Contents/Info.plist"
codesign --force --sign - "$bundle_dir"
printf '%s\n' "$bundle_dir/Contents/MacOS/mon-macos-capture"
