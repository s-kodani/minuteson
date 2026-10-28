# ASR adapters

MON selects one ASR backend per profile. Whisper and sherpa-onnx Transducer use the same model preparation, profile application, job scheduling, cancellation, checkpoint and result publication paths.

## Adding a model to an existing backend

Add a checksum-pinned model recipe to the catalog. Transducer definitions live in `app/model-catalog/asr-transducer.json`. Both ReazonSpeech variants use `sherpa-onnx-transducer`: the lightweight variant quantizes the encoder and joiner, and both variants share the FP32 decoder, tokens and Silero ONNX VAD.

The primary model has role `asr`. Backend-specific data has role `asr_asset`; executable dependencies retain `runtime_library`. The backend's `files` object maps declared slots to package-relative paths. The registry validates those slots and roles before installation or execution.

Model identity includes the backend and the complete model file set. Transport recipe or runtime changes do not change the weights' identity. Existing exact weights are copied locally after size/SHA-256 checks rather than fetched again. An already selected, checked Whisper package remains usable. Its existing engine identifier and model checksums are deliberately retained.

## Adding a backend

1. Implement `AsrAdapter` in `app/src-tauri/src/transcription/asr.rs` or a sibling module, and register it in `registry()`.
2. Declare its ID, revision, worker ID/binary, required file slots, GPU/VAD capabilities, VAD defaults, input limit and vocabulary method. Reject unsupported settings in `validate`/`prepare`.
3. Implement `recognize` and `parse_result`. Return the common `Segment` contract while retaining the native JSON artifact. Override `guarded` only when the backend needs a different acceptance procedure.
4. Add a native worker/build target if needed. Use `worker.hpp`, the canonical PCM range reader and the shared native process ownership/cancellation path. Register built binaries in the build-time worker index; runtime ASR worker names are resolved from adapter descriptors.
5. Add fixed model recipes and contract/native integration tests. Model selection UI, profile replacement and the job dispatcher must not branch on a model name.

The common configuration is `AsrConfig`. `backend: None` identifies existing Whisper weights; generic ASR packages use an explicit backend binding. The `get_asr_preferences` and `get_asr_backends` commands expose common preferences and capabilities to the frontend. The internal `whisper_profiles` settings key remains to preserve current model selections without rewriting or redownloading them.

## Native packaging

`tools/mon_inference_runtime.py` builds the Mac workers, and `mon_asr_bundle.py` assembles the small app-built Transducer runtime independently of downloaded weights. Release signing rebuilds its archive/index after signing. Windows uses the same C API and worker sources; `mon_windows_runtime.py` appends the ASR choices using packaged Windows DLLs and the signed worker.

ReazonSpeech runs on CPU on both supported targets (Mac arm64 and Windows x64). It uses Silero ONNX, not Whisper's GGML VAD. ASR inputs are capped at 30 seconds even if VAD is disabled or its detection intervals are longer. The transducer advertises no support for VAD sample overlap, so that control is omitted. Vocabulary hints use the term spelling, not readings or notes; unsupported terms are recorded in `ignoredHotwords` in the native artifact.

## Verification

- `cd app && npm run build && npm test`
- `python3 tools/mon_rust_tests.py -- asr model_ transcription analysis_flow local_engine::resident ipc_acl`
- With downloaded test assets and a voiced Japanese WAV named `japanese-voiced-test.wav` in the asset directory: `MON_ASR_ASSETS=/absolute/assets python3 tools/mon_rust_tests.py native_asr_models -- --ignored`

The native integration test exercises both precisions through the common Rust adapter and resident-worker transport, checks result parsing, and verifies a single model load across repeated requests. Windows native execution must be verified on Windows; generating or testing the packaging recipe on Mac does not establish Windows execution.
