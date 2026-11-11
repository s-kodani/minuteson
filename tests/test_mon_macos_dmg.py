import importlib.util
import io
from pathlib import Path
import sys
import tempfile
import unittest
from contextlib import redirect_stderr
from unittest.mock import patch

TOOLS = Path(__file__).resolve().parents[1] / "tools"
SPEC = importlib.util.spec_from_file_location("mon_macos_dmg", TOOLS / "mon_macos_dmg.py")
dmg = importlib.util.module_from_spec(SPEC)
sys.path.insert(0, str(TOOLS))
try:
    SPEC.loader.exec_module(dmg)
finally:
    sys.path.pop(0)


class DMGFailureTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.source = self.root / "source"
        (self.source / "MinutesOn.app").mkdir(parents=True)
        self.destination = self.root / "MinutesOn.dmg"
        self.calls = []

    def command(self, argv, **kwargs):
        self.calls.append(argv)
        if "create" in argv:
            Path(argv[-1]).write_bytes(b"writable image")
        return b"", b""

    def test_layout_failure_detaches_and_never_produces_a_distributable_image(self):
        with patch.object(dmg, "dependencies"), patch.object(dmg, "command", side_effect=self.command), \
                patch.object(dmg, "write_layout", side_effect=ValueError("layout failure")), \
                redirect_stderr(io.StringIO()), self.assertRaisesRegex(ValueError, "layout failure"):
            dmg.create_image(self.source, self.destination, "MinutesOn")
        self.assertEqual([args[1] for args in self.calls], ["create", "attach", "detach"])
        self.assertFalse(self.destination.exists())

    def test_detach_failure_preserves_working_volume_instead_of_recursively_deleting_it(self):
        def command(argv, **kwargs):
            result = self.command(argv, **kwargs)
            if "attach" in argv:
                (Path(argv[argv.index("-mountpoint") + 1]) / "keep").write_bytes(b"mounted contents")
            if "detach" in argv:
                raise ValueError("volume busy")
            return result

        with patch.object(dmg, "dependencies"), patch.object(dmg, "command", side_effect=command), \
                patch.object(dmg, "write_layout"), redirect_stderr(io.StringIO()), \
                self.assertRaisesRegex(ValueError, "volume busy"):
            dmg.create_image(self.source, self.destination, "MinutesOn")
        working = next(self.root.glob("mon-dmg-*"))
        self.assertEqual((working / "mount/keep").read_bytes(), b"mounted contents")
        self.assertTrue((working / "layout.dmg").is_file())
        self.assertFalse(self.destination.exists())


if __name__ == "__main__":
    unittest.main()
