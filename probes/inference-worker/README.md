# Job-owned inference workers

MON starts a worker lazily for a new job and keeps it until that job pauses,
finishes, fails or is cancelled. Existing jobs without `residentRuntime` retain
single-shot execution, including their retries. The optional job snapshot is
path-free and freezes the exact signed worker SHA-256. Installed workers live
under the storage root's `inference-runtimes/<sha256>/` directory and survive app
updates. Backups retain the identity, not executable bytes. A missing version
fails explicitly; MON never substitutes a different worker or execution mode.

`tools/mon_inference_runtime.py` builds offline CPU/Metal Whisper and llama
workers from the same pinned commits as the existing adapters. It patches only
model ownership and per-request reset/cleanup around the upstream CLI code.
Model-specific templates, grammar, sampling, output and bounds still use those
CLI implementations. Sherpa uses the pinned 1.13.8 C API and keeps one diarizer
and one lazily initialized feature extractor. Each feature request creates a
fresh stream. The old embedding helper remains unchanged for old jobs and voice
registration jobs.

The three static binaries and notices are packaged in
`.cache/inference-runtime/package`. Build with CMake on Apple Silicon; optional
`--whisper-source`, `--llama-source`, `--sherpa-header` and `--sherpa-license`
arguments permit a completely offline build from SHA-checked files. The macOS
build hook bundles the package. Developer ID release signing reseals each worker
and updates the runtime index in the private release copy before signing the
outer application. Source license collection does not constitute license review.

The private protocol is bounded JSON Lines on anonymous pipes, with `protocol=1`
and a monotonic request `id`. The parent sends `init`, then sequential `infer`
requests carrying `kind`, engine `args`, a canonical unit `directory` and fixed
stdout/stderr basenames. Replies are `ready`, `progress`, `done` or `error`.
`shutdown` terminates the worker. Engine output goes to the existing per-unit
files; the protocol uses a separate duplicate of stdout. A successful reply is
not a checkpoint: the parent still validates, registers and durably saves the
unit before advancing. EOF, unknown responses and mismatched IDs fail the job.

The scope is thread-local because all native job processors run synchronously
on a scheduler-owned blocking thread. It never spans jobs or replaces scheduler
ownership. `OwnedCommand` still launches the independent native guardian; its
owner channel remains separate from stdin, and closes on exec in the worker.
Cancellation drops the scope and reaps the owned group, including native calls
that cannot cooperate with cancellation. GPU trial failure alone may discard a
session and try another configuration. The successful trial stays loaded.
For llama, GPU placement identity uses the effective layer count, capped at
the loaded model's layers plus its output layer. The automatic trial's upper
bound of 256 and the saved actual count therefore share the same session when
they describe full offload. A different effective count still fails explicitly.

Run `python3 tools/mon_rust_tests.py resident_` for lifecycle, cold resume,
identity, protocol and cancellation checks. The opt-in
`tests/test_mon_inference_runtime_native.py` compares three requests (speech,
silence, speech, or distinct JSON requests) against single-shot engines. It also
covers VAD state and Metal reuse, including the automatic trial's transition
from 256 requested layers to the actual count for both Qwen 2.5 and Qwen3.
Its environment variables are documented at the
top of that file. Use only synthetic audio, matching pinned engine versions,
and `/usr/bin/time -l` in an environment allowing its resource queries. The test
records actual load counts, stable worker PIDs, output equality, elapsed time,
peak RSS and RSS after each unit. Smoke timings are not a controlled benchmark.
