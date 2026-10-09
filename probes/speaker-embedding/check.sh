#!/bin/sh
set -eu

if [ "$(uname -s)" != Darwin ]; then
  printf '%s\n' 'speaker embedding checks currently require macOS' >&2
  exit 1
fi
mon_embedding_source=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
mon_embedding_check=$(mktemp -d /private/tmp/mon-embedding-check.XXXXXX)
# Exactly the generated files in this unique owned directory, never caches or
# models from another task. Tests clean up their own small WAV fixtures.
trap 'rm -f "$mon_embedding_check/embedding-tests" "$mon_embedding_check/mon-speaker-embedding" "$mon_embedding_check/fake-sherpa.dylib" "$mon_embedding_check/wrong-version.dylib" "$mon_embedding_check/missing-symbol.dylib"; rmdir "$mon_embedding_check"' EXIT
sh "$mon_embedding_source/build.sh" --out-dir "$mon_embedding_check"
rustfmt --edition 2021 --check "$mon_embedding_source/main.rs" "$mon_embedding_source/fake_sherpa.rs"
rustc --edition=2021 --crate-type cdylib "$mon_embedding_source/fake_sherpa.rs" -o "$mon_embedding_check/fake-sherpa.dylib"
rustc --edition=2021 --crate-type cdylib --cfg mon_fake_wrong_version "$mon_embedding_source/fake_sherpa.rs" -o "$mon_embedding_check/wrong-version.dylib"
rustc --edition=2021 --crate-type cdylib --cfg mon_fake_missing_symbol "$mon_embedding_source/fake_sherpa.rs" -o "$mon_embedding_check/missing-symbol.dylib"
rustc --edition=2021 --test -C debug-assertions=yes -C overflow-checks=yes "$mon_embedding_source/main.rs" -o "$mon_embedding_check/embedding-tests"
env MON_EMBEDDING_HELPER="$mon_embedding_check/mon-speaker-embedding" \
  MON_EMBEDDING_FAKE_LIBRARY="$mon_embedding_check/fake-sherpa.dylib" \
  MON_EMBEDDING_WRONG_VERSION_LIBRARY="$mon_embedding_check/wrong-version.dylib" \
  MON_EMBEDDING_MISSING_SYMBOL_LIBRARY="$mon_embedding_check/missing-symbol.dylib" \
  "$mon_embedding_check/embedding-tests" --include-ignored --skip real_native_model
# Exercise the actual parent runner too, including owned child kill/wait.
env MON_EMBEDDING_HELPER="$mon_embedding_check/mon-speaker-embedding" \
  MON_EMBEDDING_FAKE_LIBRARY="$mon_embedding_check/fake-sherpa.dylib" \
  python3 "$mon_embedding_source/../../tools/mon_rust_tests.py" \
  diarization::embedding::tests::feature_child_verifies_fixed_inputs_and_reaps_on_cancellation_timeout_overflow_and_failure -- --ignored
env MON_EMBEDDING_HELPER="$mon_embedding_check/mon-speaker-embedding" \
  MON_EMBEDDING_FAKE_LIBRARY="$mon_embedding_check/fake-sherpa.dylib" \
  python3 "$mon_embedding_source/../../tools/mon_rust_tests.py" \
  diarization::known::tests::saved_run_worker_ -- --ignored
