.PHONY: check test lint build-macos-probe check-app check-caf-playback check-capture-clock

check: lint test build-macos-probe check-app check-caf-playback check-capture-clock

check-app:
	npm --prefix app run build
	npm --prefix app test
	cd app/src-tauri && cargo fmt --check && cargo test --locked --offline

check-caf-playback:
	@mon_preview_dir=$$(mktemp -d /tmp/mon-caf-preview.XXXXXX) || exit 1; \
	trap 'rm -f "$$mon_preview_dir/source.caf" "$$mon_preview_dir/result.caf"; rmdir "$$mon_preview_dir"' EXIT; \
	env SWIFTPM_MODULECACHE_OVERRIDE=/tmp/mon-swift-module-cache CLANG_MODULE_CACHE_PATH=/tmp/mon-clang-module-cache swift probes/macos-capture/Tests/CafWriterSmoke.swift --preview-fixtures "$$mon_preview_dir" && \
	env MON_PREVIEW_FIXTURE_DIR="$$mon_preview_dir" cargo test --locked --offline --manifest-path app/src-tauri/Cargo.toml avfoundation_generated_caf -- --ignored

check-capture-clock:
	@mon_clock_dir=$$(mktemp -d /tmp/mon-capture-clock.XXXXXX) || exit 1; \
	trap 'rm -f "$$mon_clock_dir/manifest.json" "$$mon_clock_dir/microphone.caf" "$$mon_clock_dir/system.caf"; rmdir "$$mon_clock_dir"' EXIT; \
	env MON_CAPTURE_CLOCK_FIXTURE_DIR="$$mon_clock_dir" SWIFTPM_MODULECACHE_OVERRIDE=/tmp/mon-swift-module-cache CLANG_MODULE_CACHE_PATH=/tmp/mon-clang-module-cache swift test --package-path probes/macos-capture --disable-sandbox --filter CaptureClockFixtureTests/testSwiftCaptureClockFixtureRoundTripsRealPCMAndExportsActualManifest && \
	env MON_CAPTURE_CLOCK_FIXTURE_DIR="$$mon_clock_dir" cargo test --locked --offline --manifest-path app/src-tauri/Cargo.toml swift_generated_capture_clock_manifest -- --ignored

test:
	python3 -m unittest discover -s tests -v

lint:
	python3 -m compileall -q tools tests
	python3 tools/mon_eval.py validate
	plutil -lint probes/macos-capture/Info.plist
	plutil -lint probes/macos-capture/PrivacyInfo.xcprivacy
	cd probes/macos-capture && swift format lint --strict -r Sources Tests Package.swift

build-macos-probe:
	cd probes/macos-capture && env SWIFTPM_MODULECACHE_OVERRIDE=/tmp/mon-swift-module-cache CLANG_MODULE_CACHE_PATH=/tmp/mon-clang-module-cache swift build --disable-sandbox
	cd probes/macos-capture && env SWIFTPM_MODULECACHE_OVERRIDE=/tmp/mon-swift-module-cache CLANG_MODULE_CACHE_PATH=/tmp/mon-clang-module-cache swift test --disable-sandbox
	cd probes/macos-capture && env SWIFTPM_MODULECACHE_OVERRIDE=/tmp/mon-swift-module-cache CLANG_MODULE_CACHE_PATH=/tmp/mon-clang-module-cache swift Tests/CafWriterSmoke.swift
