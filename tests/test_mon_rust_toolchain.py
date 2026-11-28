import re
from pathlib import Path
import tomllib
import unittest

import yaml


ROOT = Path(__file__).resolve().parents[1]


class RustToolchainTests(unittest.TestCase):
    def test_workflows_install_the_repository_toolchain(self):
        with (ROOT / "rust-toolchain.toml").open("rb") as config:
            channel = tomllib.load(config)["toolchain"]["channel"]
        self.assertRegex(channel, r"^\d+\.\d+\.\d+$")
        workflows = ROOT / ".github/workflows"
        pins = set()
        for path in sorted([*workflows.glob("*.yml"), *workflows.glob("*.yaml")]):
            workflow = yaml.safe_load(path.read_text(encoding="utf8"))
            for job_name, job in workflow["jobs"].items():
                for step in job.get("steps", []):
                    action = step.get("uses", "")
                    if not action.startswith("dtolnay/rust-toolchain@"):
                        continue
                    with self.subTest(workflow=path.name, job=job_name):
                        pin = action.split("@", 1)[1]
                        self.assertIsNotNone(re.fullmatch(r"[0-9a-f]{40}", pin))
                        self.assertEqual(step.get("with", {}).get("toolchain"), channel)
                        pins.add(pin)
        self.assertEqual(len(pins), 1, "CI の Rust セットアップには同じコミットの固定が必要")
