import importlib.util
from pathlib import Path
import tempfile
import unittest
import wave

spec = importlib.util.spec_from_file_location('meeting_benchmark', Path(__file__).resolve().parents[1] / 'tools/mon_meeting_audio_benchmark.py')
benchmark = importlib.util.module_from_spec(spec)
spec.loader.exec_module(benchmark)


class MeetingBenchmarkInputTests(unittest.TestCase):
    def fixture(self, root, rate=16000):
        source = root / 'source.wav'
        pcm = b'\x01\x00\xfe\xff' * 50
        with wave.open(str(source), 'wb') as out:
            out.setparams((1, 2, rate, 100, 'NONE', 'not compressed'))
            out.writeframes(pcm)
        return source, pcm

    def test_repeated_wave_keeps_exact_samples_and_duration(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source, pcm = self.fixture(root)
            target = root / 'meeting.wav'
            benchmark.make_wave(source, target, 1, True)
            with wave.open(str(target), 'rb') as audio:
                self.assertEqual((audio.getnframes(), audio.getframerate(), audio.getnchannels()), (16000, 16000, 1))
                self.assertEqual(audio.readframes(16000), pcm * 160)

    def test_short_input_requires_explicit_repetition_and_output_is_exclusive(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source, _ = self.fixture(root)
            target = root / 'meeting.wav'
            with self.assertRaisesRegex(ValueError, 'too short'):
                benchmark.make_wave(source, target, 1, False)
            self.assertFalse(target.exists())
            target.write_bytes(b'owned existing file')
            with self.assertRaises(FileExistsError):
                benchmark.make_wave(source, target, 1, True)
            self.assertEqual(target.read_bytes(), b'owned existing file')

    def test_wrong_format_is_not_silently_converted(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source, _ = self.fixture(root, 48000)
            with self.assertRaisesRegex(ValueError, 'mono PCM16 16 kHz'):
                benchmark.make_wave(source, root / 'meeting.wav', 1, True)


if __name__ == '__main__':
    unittest.main()
