# ReazonSpeech / ASR adapters verification — 2026-10-28

Branch: `codex/asr-adapters-reazonspeech`, based on `main` at `149a4ba`.

## Implemented

- Common `AsrConfig` and built-in `AsrAdapter` registry for Whisper and sherpa-onnx Transducer.
- Shared ASR model-selection slot, settings API/capabilities, immutable model references, job lifecycle and result publication.
- ReazonSpeech-k2-v2 INT8 encoder/joiner + FP32 decoder, and full FP32 variants, pinned to model revision `291488c8151be24d7da4bf7af26e533fad96e407` with independently checked file sizes/SHA-256.
- Transducer CPU worker using the pinned sherpa-onnx 1.13.8 C API, canonical PCM range reader and common resident-worker transport.
- Explicit VAD capabilities/defaults. Unsupported overlap settings are omitted in the UI and rejected by native settings validation. Whisper defaults remain unchanged.
- Exact downloaded weights remain usable; a changed runtime recipe can reuse local model bytes without contacting a model source.
- Mac packaging/signing and Windows native packaging recipes include the new worker. Backends are not automatically selected or substituted.

## Checks

- `cd app && npm run build`: passed.
- `cd app && npm test`: 835 passed, 1 skipped.
- `python3 tools/mon_rust_tests.py --jobs 4 --threads 4 -- asr model_ transcription analysis_flow local_engine::resident ipc_acl`: 374 passed, 23 ignored. Local HTTP fixture tests required execution outside the network-restricted sandbox.
- Final focused IPC/ASR/bundle checks after registering the new Tauri commands: 16 passed, 1 ignored.
- Opt-in native integration with `MON_ASR_ASSETS`: both actual model variants transcribed a synthetic voiced Japanese WAV through the common Rust adapter and resident transport. Repeated requests returned identical segments and logged exactly one model load per model session. The test passed again after the final native rebuild.
- Packaging unit tests: 2 passed, including Windows catalog/DLL assembly and preservation of model hashes.
- Mac release/signing unit tests: 33 passed.
- Existing Whisper runtime builder tests: 21 passed.
- Changed Python builders compile successfully; `git diff --check` passed.

The Japanese input was generated locally using the installed Kyoko voice, converted to mono PCM16 at 16kHz (about 9.57 seconds). No private meeting recordings were used. These are functional checks, not a comparative accuracy or performance benchmark.

## Limits

Windows native compilation and execution were not available on this Mac. Windows validation covers the packaging transformation and shared sources, not a Windows runtime result. Historical job/backup compatibility and cross-OS data migration were outside this change's scope.

Subsequent Windows compilation, real-model execution, fixes and regression checks are recorded in [the Windows verification](reazonspeech-windows-2026-10-29.md).
