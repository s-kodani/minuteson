.DEFAULT_GOAL := check

.PHONY: check test lint build-macos-probe check-app check-browser prepare-capture-core check-caf-playback check-capture-clock check-input-levels check-input-status check-input-history check-input-journal check-capture-test check-microphone-recovery check-system-recovery check-capture-ending check-pc-only-capture check-recording-raw-prefix check-speaker-embedding build-rust-tests test-rust test-rust-focused rust-test-cache

build-rust-tests:
	python3 tools/mon_rust_tests.py --build-only

test-rust:
	python3 tools/mon_rust_tests.py

test-rust-focused:
	python3 tools/mon_rust_tests.py "$(RUST_TEST_FILTER)"

rust-test-cache:
	python3 tools/mon_rust_tests.py --status

check: lint test build-macos-probe check-app check-browser check-caf-playback check-capture-clock check-input-levels check-input-status check-input-history check-input-journal check-capture-test check-microphone-recovery check-system-recovery check-capture-ending check-pc-only-capture check-recording-raw-prefix check-speaker-embedding

prepare-capture-core:
	sh probes/macos-capture/build-core.sh

check-pc-only-capture: prepare-capture-core
	python3 tools/mon_rust_tests.py swift_generated_pc_only_capture -- --ignored

check-recording-raw-prefix: prepare-capture-core
	python3 tools/mon_rust_tests.py swift_dense_capture_clock_roundtrip_finalizes_replays_and_restores_over_legacy_capacity -- --ignored
	python3 tools/mon_rust_tests.py swift_generated_recording_segments_bind_actual_storage_and_encode_flac_without_rewriting_raw -- --ignored
	python3 tools/mon_rust_tests.py partial_recovery_owned_sigkill_checkpoints_keep_interrupted_and_require_fresh_consent -- --ignored
	python3 tools/mon_rust_tests.py flac_relocation_owned_sigkill_cold_resume_keeps_receipts_and_unknown_files -- --ignored
	python3 tools/mon_rust_tests.py flac_relocation_partial_owned_sigkill_resumes_with_fresh_consent_and_current_document -- --ignored

check-input-journal: prepare-capture-core
	@mon_journal_dir=$$(mktemp -d /tmp/mon-input-journal.XXXXXX) || exit 1; \
	trap 'rm -f "$$mon_journal_dir/input-events.json" "$$mon_journal_dir/input-journal-status.json"; rmdir "$$mon_journal_dir"' EXIT; \
	env MON_INPUT_JOURNAL_FIXTURE_DIR="$$mon_journal_dir" SWIFTPM_MODULECACHE_OVERRIDE=/tmp/mon-swift-module-cache CLANG_MODULE_CACHE_PATH=/tmp/mon-clang-module-cache swift test --package-path probes/macos-capture --disable-sandbox --filter InputEventJournalTests/testPolicyTransitionsAreRetainedWithoutHeartbeatAndSealedWithoutRendering && \
	env MON_INPUT_JOURNAL_FIXTURE_DIR="$$mon_journal_dir" python3 tools/mon_rust_tests.py swift_generated_input_journal -- --ignored

check-input-history: prepare-capture-core
	@mon_history_dir=$$(mktemp -d /tmp/mon-input-history.XXXXXX) || exit 1; \
	trap 'rm -f "$$mon_history_dir/input-status.json" "$$mon_history_dir/input-notices.json" "$$mon_history_dir/input-notices.lock"; rmdir "$$mon_history_dir"' EXIT; \
	env MON_INPUT_HISTORY_FIXTURE_DIR="$$mon_history_dir" SWIFTPM_MODULECACHE_OVERRIDE=/tmp/mon-swift-module-cache CLANG_MODULE_CACHE_PATH=/tmp/mon-clang-module-cache swift test --package-path probes/macos-capture --disable-sandbox --filter InputEventHistoryTests/testBothRapidTransitionsSurviveOneHeartbeatAndExportForRust && \
	env MON_INPUT_HISTORY_FIXTURE_DIR="$$mon_history_dir" python3 tools/mon_rust_tests.py swift_generated_input_history -- --ignored

check-speaker-embedding:
	sh probes/speaker-embedding/check.sh

check-capture-ending: prepare-capture-core
	python3 tools/mon_rust_tests.py swift_generated_capture_ending -- --ignored

check-system-recovery: prepare-capture-core
	@mon_system_dir=$$(mktemp -d /tmp/mon-system-recovery.XXXXXX) || exit 1; \
	trap 'rm -f "$$mon_system_dir/manifest.json" "$$mon_system_dir/microphone.caf" "$$mon_system_dir/system.caf"; rmdir "$$mon_system_dir"' EXIT; \
	env MON_SYSTEM_RECOVERY_FIXTURE_DIR="$$mon_system_dir" SWIFTPM_MODULECACHE_OVERRIDE=/tmp/mon-swift-module-cache CLANG_MODULE_CACHE_PATH=/tmp/mon-clang-module-cache swift test --package-path probes/macos-capture --disable-sandbox --filter SystemAudioRecoveryTests/testNormalRecorderRebindsWithoutAdoptingOldErrorsOrPCMAndExportsRealCAF && \
	env MON_SYSTEM_RECOVERY_FIXTURE_DIR="$$mon_system_dir" python3 tools/mon_rust_tests.py swift_system_rebinding_retains_pcm_gap_and_continuous_microphone -- --ignored

check-input-status: prepare-capture-core
	@mon_status_dir=$$(mktemp -d /tmp/mon-input-status.XXXXXX) || exit 1; \
	trap 'rm -f "$$mon_status_dir/input-status.json" "$$mon_status_dir/input-notices.json" "$$mon_status_dir/input-notices.lock"; rmdir "$$mon_status_dir"' EXIT; \
	env MON_INPUT_STATUS_FIXTURE_DIR="$$mon_status_dir" SWIFTPM_MODULECACHE_OVERRIDE=/tmp/mon-swift-module-cache CLANG_MODULE_CACHE_PATH=/tmp/mon-clang-module-cache swift test --package-path probes/macos-capture --disable-sandbox --filter InputStatusTests/testSwiftBindingHeartbeatPublishesActualPolicyAndExportsForNativeReader && \
	env MON_INPUT_STATUS_FIXTURE_DIR="$$mon_status_dir" python3 tools/mon_rust_tests.py swift_generated_input_status -- --ignored

check-app:
	python3 tools/mon_contracts.py --check
	python3 tools/mon_model_catalog.py --check
	npm --prefix app run check:contracts
	npm --prefix app run build
	npm --prefix app test
	cargo test --workspace --exclude mon-desktop --locked --lib
	cd app/src-tauri && cargo fmt --check
	python3 tools/mon_rust_tests.py

check-browser:
	npm --prefix app run test:browser

check-caf-playback: prepare-capture-core
	@mon_preview_dir=$$(mktemp -d /tmp/mon-caf-preview.XXXXXX) || exit 1; \
	trap 'rm -f "$$mon_preview_dir/source.caf" "$$mon_preview_dir/result.caf"; rmdir "$$mon_preview_dir"' EXIT; \
	env SWIFTPM_MODULECACHE_OVERRIDE=/tmp/mon-swift-module-cache CLANG_MODULE_CACHE_PATH=/tmp/mon-clang-module-cache swift probes/macos-capture/Tests/CafWriterSmoke.swift --preview-fixtures "$$mon_preview_dir" && \
	env MON_PREVIEW_FIXTURE_DIR="$$mon_preview_dir" python3 tools/mon_rust_tests.py avfoundation_generated_caf -- --ignored

check-capture-clock: prepare-capture-core
	@mon_clock_dir=$$(mktemp -d /tmp/mon-capture-clock.XXXXXX) || exit 1; \
	trap 'rm -f "$$mon_clock_dir/manifest.json" "$$mon_clock_dir/microphone.caf" "$$mon_clock_dir/system.caf"; rmdir "$$mon_clock_dir"' EXIT; \
	env MON_CAPTURE_CLOCK_FIXTURE_DIR="$$mon_clock_dir" SWIFTPM_MODULECACHE_OVERRIDE=/tmp/mon-swift-module-cache CLANG_MODULE_CACHE_PATH=/tmp/mon-clang-module-cache swift test --package-path probes/macos-capture --disable-sandbox --filter CaptureClockFixtureTests/testSwiftCaptureClockFixtureRoundTripsRealPCMAndExportsActualManifest && \
	env MON_CAPTURE_CLOCK_FIXTURE_DIR="$$mon_clock_dir" python3 tools/mon_rust_tests.py swift_generated_capture_clock_manifest -- --ignored

check-input-levels: prepare-capture-core
	@mon_level_dir=$$(mktemp -d /tmp/mon-input-level.XXXXXX) || exit 1; \
	trap 'rm -f "$$mon_level_dir/input-levels.json"; rmdir "$$mon_level_dir"' EXIT; \
	env MON_INPUT_LEVEL_FIXTURE_DIR="$$mon_level_dir" SWIFTPM_MODULECACHE_OVERRIDE=/tmp/mon-swift-module-cache CLANG_MODULE_CACHE_PATH=/tmp/mon-clang-module-cache swift test --package-path probes/macos-capture --disable-sandbox --filter InputLevelTests/testSwiftLevelPublicationAndManifestExclusion && \
	env MON_INPUT_LEVEL_FIXTURE_DIR="$$mon_level_dir" python3 tools/mon_rust_tests.py swift_generated_input_levels -- --ignored

check-capture-test: prepare-capture-core
	@mon_test_dir=$$(mktemp -d /tmp/mon-capture-test.XXXXXX) || exit 1; \
	trap 'rm -f "$$mon_test_dir/manifest.json" "$$mon_test_dir/microphone.caf" "$$mon_test_dir/system.caf"; rmdir "$$mon_test_dir"' EXIT; \
	env MON_CAPTURE_TEST_FIXTURE_DIR="$$mon_test_dir" SWIFTPM_MODULECACHE_OVERRIDE=/tmp/mon-swift-module-cache CLANG_MODULE_CACHE_PATH=/tmp/mon-clang-module-cache swift test --package-path probes/macos-capture --disable-sandbox --filter CaptureTestWindowTests/testSwiftFiveSecondWindowWritesOnlySelectedFramesAndExportsPrivateManifest && \
	env MON_CAPTURE_TEST_FIXTURE_DIR="$$mon_test_dir" python3 tools/mon_rust_tests.py swift_generated_five_second_test -- --ignored

check-microphone-recovery: prepare-capture-core
	@mon_recovery_dir=$$(mktemp -d /tmp/mon-microphone-recovery.XXXXXX) || exit 1; \
	trap 'rm -f "$$mon_recovery_dir/manifest.json" "$$mon_recovery_dir/microphone.caf" "$$mon_recovery_dir/system.caf"; rmdir "$$mon_recovery_dir"' EXIT; \
	env MON_MICROPHONE_RECOVERY_FIXTURE_DIR="$$mon_recovery_dir" SWIFTPM_MODULECACHE_OVERRIDE=/tmp/mon-swift-module-cache CLANG_MODULE_CACHE_PATH=/tmp/mon-clang-module-cache swift test --package-path probes/macos-capture --disable-sandbox --filter MicrophoneRecoveryTests/testSyntheticCAFAppendsAcrossVerifiedBindingsWithoutOverwritingOrPadding && \
	env MON_MICROPHONE_RECOVERY_FIXTURE_DIR="$$mon_recovery_dir" python3 tools/mon_rust_tests.py swift_microphone_rebinding_retains_pcm_gap_and_other_track_through_database_finalization -- --ignored

test:
	python3 -m unittest discover -s tests -v

lint:
	python3 -m compileall -q tools tests
	python3 tools/mon_eval.py validate
	plutil -lint probes/macos-capture/Info.plist
	plutil -lint probes/macos-capture/PrivacyInfo.xcprivacy
	cd probes/macos-capture && swift format lint --strict -r Sources Tests Package.swift

build-macos-probe: prepare-capture-core
	cd probes/macos-capture && env SWIFTPM_MODULECACHE_OVERRIDE=/tmp/mon-swift-module-cache CLANG_MODULE_CACHE_PATH=/tmp/mon-clang-module-cache swift build --disable-sandbox
	cd probes/macos-capture && env SWIFTPM_MODULECACHE_OVERRIDE=/tmp/mon-swift-module-cache CLANG_MODULE_CACHE_PATH=/tmp/mon-clang-module-cache swift test --disable-sandbox
	cd probes/macos-capture && env SWIFTPM_MODULECACHE_OVERRIDE=/tmp/mon-swift-module-cache CLANG_MODULE_CACHE_PATH=/tmp/mon-clang-module-cache swift Tests/CafWriterSmoke.swift

.PHONY: check-docs check-security

check-docs:
	python3 tools/mon_docs.py

check-security:
	python3 tools/mon_ci_tools.py
	.cache/security-tools/actionlint -shellcheck=''
	.cache/security-tools/gitleaks git --redact=100 --config .gitleaks.toml
	python3 tools/mon_security.py scan
