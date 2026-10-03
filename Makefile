.PHONY: check test lint build-macos-probe check-app

check: lint test build-macos-probe check-app

check-app:
	npm --prefix app run build
	npm --prefix app test
	cd app/src-tauri && cargo fmt --check && cargo test --locked --offline

test:
	python3 -m unittest discover -s tests -v

lint:
	python3 -m compileall -q tools tests
	python3 tools/mon_eval.py validate
	plutil -lint probes/macos-capture/Info.plist
	cd probes/macos-capture && swift format lint --strict -r Sources Tests Package.swift

build-macos-probe:
	cd probes/macos-capture && env SWIFTPM_MODULECACHE_OVERRIDE=/tmp/mon-swift-module-cache CLANG_MODULE_CACHE_PATH=/tmp/mon-clang-module-cache swift build --disable-sandbox
	cd probes/macos-capture && env SWIFTPM_MODULECACHE_OVERRIDE=/tmp/mon-swift-module-cache CLANG_MODULE_CACHE_PATH=/tmp/mon-clang-module-cache swift Tests/CafWriterSmoke.swift
