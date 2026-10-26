# Audio import parity fixtures

These files contain synthetic data, with no third-party recordings. Both native
container validators test the same corpus; `manifest.json` records its expected
results and hashes. CI requires no FFmpeg or network.
`structure-*` are deliberately minimal ISO BMFF boxes, for validation only.
`chapters-aac.m4a` and `chapters-alac.m4a` contain a generated 440 Hz, 48 kHz,
mono, one-second sine and two referenced QuickTime text chapters. Decode tests
must preserve the original bytes and extract only the sound track.

Regenerate with `python3 tests/fixtures/audio-import/generate.py` and FFmpeg.
The generator records byte sizes and SHA-256 digests in the manifest. Compressed
bytes can differ across FFmpeg versions; review regenerated files and hashes.
The initial corpus was generated with FFmpeg 8.0.1 (Homebrew).
