import shutil
import struct
import subprocess
import tempfile
import unittest
import wave
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


class WaveWriterTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        compiler = shutil.which("c++")
        if compiler is None:
            raise unittest.SkipTest("C++ compiler is unavailable")
        cls.temp = tempfile.TemporaryDirectory()
        cls.executable = Path(cls.temp.name) / "wave-writer-fixture"
        subprocess.run(
            [compiler, "-std=c++20", "-Wall", "-Wextra", "-Werror",
             "-I", str(ROOT / "probes/windows-capture"),
             str(ROOT / "tests/wave_writer_fixture.cpp"), "-o", str(cls.executable)],
            check=True, capture_output=True, text=True,
        )

    @classmethod
    def tearDownClass(cls):
        if hasattr(cls, "temp"):
            cls.temp.cleanup()

    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        subprocess.run([str(self.executable), self.directory.name], check=True)
        self.root = Path(self.directory.name)

    def test_segments_and_silence_have_valid_wave_headers(self):
        files = sorted(self.root.glob("split-*.wav"))
        self.assertEqual([p.name for p in files], ["split-0000.wav", "split-0001.wav"])
        with wave.open(str(files[0])) as first:
            self.assertEqual(first.getparams()[:3], (2, 2, 48000))
            self.assertEqual(first.readframes(2), bytes(range(1, 9)))
        with wave.open(str(files[1])) as second:
            self.assertEqual(second.getnframes(), 1)
            self.assertEqual(second.readframes(1), b"\0" * 4)
        for path in files:
            data = path.read_bytes()
            self.assertEqual(struct.unpack_from("<I", data, 4)[0], len(data) - 8)

    def test_odd_data_and_format_sizes_are_padded(self):
        odd_data = (self.root / "odd-data-0000.wav").read_bytes()
        self.assertEqual(struct.unpack_from("<I", odd_data, 40)[0], 1)
        self.assertEqual(odd_data[-2:], b"\x7f\0")
        self.assertEqual(struct.unpack_from("<I", odd_data, 4)[0], len(odd_data) - 8)
        odd_format = (self.root / "odd-format-0000.wav").read_bytes()
        self.assertEqual(struct.unpack_from("<I", odd_format, 16)[0], 17)
        self.assertEqual(odd_format[37], 0)
        self.assertEqual(odd_format[38:42], b"data")
        self.assertEqual(struct.unpack_from("<I", odd_format, 42)[0], 1)
        self.assertEqual(struct.unpack_from("<I", odd_format, 4)[0], len(odd_format) - 8)


if __name__ == "__main__":
    unittest.main()
