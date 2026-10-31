# ASR adapters

MON selects one ASR backend per profile. Whisper, character transducers and NeMo TDT use the same model preparation, profile application, job scheduling, cancellation, checkpoint and result publication paths. Backends are registered at build time; adding a backend requires rebuilding the app.

## Adding a model to an existing backend

Add a checksum-pinned model definition to `app/model-catalog/model-definitions.json`, then regenerate the platform catalog with `python tools/mon_model_catalog.py`. Both ReazonSpeech variants use `sherpa-onnx-transducer`: the lightweight variant quantizes the encoder and joiner, and both variants share the FP32 decoder, tokens and Silero ONNX VAD. Haas uses `sherpa-onnx-nemo-tdt` with a quantized encoder and external `encoder.data`, FP32 decoder/joiner and BPE hotword vocabulary.

The primary model has role `asr`. Backend-specific data has role `asr_asset`; executable dependencies retain `runtime_library`. The backend's `files` object maps declared slots to package-relative paths. The registry validates those slots and roles before installation or execution.

Model identity includes the backend and the complete model file set. Transport recipe or runtime changes do not change the weights' identity. Existing exact weights are copied locally after size/SHA-256 checks rather than fetched again. An already selected, checked Whisper package remains usable. Its existing engine identifier and model checksums are deliberately retained.

## Adding a backend

1. Implement `AsrAdapter` in `app/src-tauri/src/transcription/asr.rs` or a sibling module, and register it in `registry()`.
2. Declare its ID, revision, worker ID/binary, required file slots, GPU/VAD capabilities, VAD defaults, input limit and vocabulary method. Reject unsupported settings in `validate`/`prepare`.
3. Implement `recognize` and `parse_result`. Return the common `Segment` contract while retaining the native JSON artifact. Override `guarded` only when the backend needs a different acceptance procedure.
4. Add a native worker/build target if needed. Use `worker.hpp`, the canonical PCM range reader and the shared native process ownership/cancellation path. Register built binaries in the build-time worker index; runtime ASR worker names are resolved from adapter descriptors.
5. Add fixed model recipes and contract/native integration tests. Model selection UI, profile replacement and the job dispatcher must not branch on a model name.

`native_options()` supplies typed model/feature/tokenization/segment-limit settings to the common transducer worker. Legacy adapters return `None`, retaining their original command lines and native defaults. `compatibility_hint()` declares an additional real decoder smoke check with VAD disabled; its input, output and logs are sealed in the compatibility report alongside the ordinary trial. Existing backends retain their original report fields and artifact lists.

The common configuration is `AsrConfig`. `backend: None` identifies existing Whisper weights; generic ASR packages use an explicit backend binding. The `get_asr_preferences` and `get_asr_backends` commands expose common preferences and capabilities to the frontend. The internal `whisper_profiles` settings key remains to preserve current model selections without rewriting or redownloading them.

## Native packaging

`tools/mon_inference_runtime.py` builds the Mac workers, and `mon_asr_bundle.py` assembles the small app-built Transducer runtime independently of downloaded weights. Release signing rebuilds its archive/index after signing. Windows uses the same C API and worker sources; `mon_windows_runtime.py` appends the ASR choices using packaged Windows DLLs and the signed worker.

ReazonSpeech runs on CPU on both supported targets (Mac arm64 and Windows x64). It uses Silero ONNX, not Whisper's GGML VAD. ASR inputs are capped at 30 seconds even if VAD is disabled or its detection intervals are longer. The transducer advertises no support for VAD sample overlap, so that control is omitted. Vocabulary hints use the term spelling, not readings or notes; unsupported terms are recorded in `ignoredHotwords` in the native artifact.

Haas also runs on CPU with Silero VAD, a 128-dimensional NeMo feature configuration and a hard 29-second decoding limit. `encoder.data` must retain its name beside the encoder and is fingerprinted like every other model asset. With usable spelling hints, decoding uses modified beam search (width 4, hotword score 1.5); otherwise it uses greedy search. BPE separators and byte-fallback tokens are decoded before publishing UTF-8 segments. Model availability and a successful compatibility check do not establish recognition quality.

The small Haas hotword vocabulary is reproduced by `python tools/mon_asr_assets.py --check` from checksum-pinned tokens. Equal scores follow the upstream NeMo TDT example; this is a longest-match hint encoder, not the original SentencePiece model. Source, algorithm, output hash and attribution live in `third-party/asr-assets/`. Original unquantized model weights are not required. The ASR runtime index keeps its shared worker/notices and optionally declares `modelAssets` by catalog choice ID. Only that choice receives the declared asset; old indexes still expose their original ReazonSpeech choices. Windows binds the same assets to its existing runtime archive. Model asset changes are repackaged without changing old weights' identities.

The stock BPE hotword encoder inserts an initial word boundary, which prevented a Japanese spelling hint from matching mid-sentence. The worker therefore computes both boundary variants and carries the resulting token IDs through a reversible, worker-owned symbol table using the stock character hotword API. `<unk>` and `<blk>` keep their original identities; every other token, including byte-fallback tokens, has a unique private-use symbol. Neural weights, token IDs and decoder shapes stay unchanged. Returned symbols are mapped back before token artifacts and displayed text are built; upstream symbol text must never be published directly. The temporary table is created exclusively in the private unit workspace and removed on normal exit. Upstream upgrades must retain the full no-hint token/timestamp/JSON equivalence test and the frozen mid-sentence spelling test.

The character vocabulary reader accepts both tab and space separators. This fixes the previous ReazonSpeech path that silently ignored valid spelling hints in its tab-separated token table. New worker calls retain the old automatic decoder-selection policy and no-hint output; existing jobs continue to use their frozen old worker bytes. Spellings containing reserved hotword syntax are rejected as literal unsupported terms rather than interpreted as weighting instructions or separate phrases.

Dictionary info is shared across common, meeting and import editors. Per-model support is displayed in installed-copy details using backend descriptors: Whisper uses spelling/readings as prompt text; ReazonSpeech and Haas use spelling only. Notes are retained for display. Unknown backend information is shown as unavailable rather than inferred from the model name.

Existing selected models, stored vocabulary and fixed jobs are retained. A fixed job keeps its original worker hash; missing historical bytes fail explicitly. Supported older backups remain readable by the new app, while older apps cannot execute or validate the newly introduced Haas backend.

## Verification

- `cd app && npm run build && npm test`
- `python3 tools/mon_rust_tests.py -- asr model_ transcription analysis_flow local_engine::resident ipc_acl`
- With downloaded test assets and a voiced Japanese WAV named `japanese-voiced-test.wav` in the asset directory: `MON_ASR_ASSETS=/absolute/assets python3 tools/mon_rust_tests.py native_asr_models -- --ignored`

The native integration test exercises both precisions through the common Rust adapter and resident-worker transport, checks result parsing, and verifies a single model load across repeated requests. Windows native execution must be verified on Windows; generating or testing the packaging recipe on Mac does not establish Windows execution.
