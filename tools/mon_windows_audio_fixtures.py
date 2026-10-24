"""Independent audio import fixtures. Never records or downloads media.

Pass an existing FFmpeg executable and a NEW output folder, then set
MON_WINDOWS_IMPORT_FIXTURES to the output when running the ignored Rust test.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess

def generate(ffmpeg: Path, output: Path):
    ffmpeg = ffmpeg.resolve(strict=True)
    output.mkdir(parents=True, exist_ok=False)
    output = output.resolve(strict=True)
    def run(*arguments):
        subprocess.run([str(ffmpeg), "-nostdin", "-v", "error", *map(str, arguments)], check=True)
    for name, codec, container in [
        ("pcm.wav", "pcm_s16le", "wav"), ("float.wav", "pcm_f32le", "wav"),
        ("audio.mp3", "libmp3lame", "mp3"), ("audio.flac", "flac", "flac"),
        ("aac.m4a", "aac", "ipod"), ("alac.m4a", "alac", "ipod"),
    ]:
        run("-f", "lavfi", "-i", "sine=frequency=440:duration=4", "-ar", 48000,
            "-ac", 2, "-c:a", codec, "-f", container, output / name)
    run("-f", "lavfi", "-i", "sine=duration=1", "-f", "lavfi", "-i",
        "color=size=16x16:rate=1:duration=1", "-c:a", "aac", "-c:v", "mpeg4",
        "-shortest", output / "video.m4a")
    run("-f", "lavfi", "-i", "sine=duration=1", "-map", "0:a", "-map", "0:a",
        "-c:a", "aac", output / "two-tracks.m4a")
    version = subprocess.check_output([str(ffmpeg), "-version"]).decode("utf8").splitlines()[0]
    manifest = {"generator": version, "files": {}}
    for path in sorted(output.iterdir()):
        with path.open("rb") as stream:
            manifest["files"][path.name] = {"bytes": path.stat().st_size, "sha256": hashlib.file_digest(stream, "sha256").hexdigest()}
    (output / "fixture-manifest.json").write_text(json.dumps(manifest, indent=2)+"\n", encoding="utf8")
    print(output)

if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--ffmpeg", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    arguments = parser.parse_args()
    generate(arguments.ffmpeg, arguments.output)
