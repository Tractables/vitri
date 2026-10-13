"""Release-version checks must cover the standalone consumer example."""

import re
import runpy
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).with_name("check-versions.py")
ROOT = SCRIPT.parents[2]
EXAMPLE = "examples/rsdd-count/"


class ExampleVersions(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        sources = runpy.run_path(str(SCRIPT))["SOURCES"]
        for source in {*sources, EXAMPLE + "Cargo.toml", EXAMPLE + "Cargo.lock",
                       ".github/scripts/check-versions.py"}:
            target = self.root / source
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / source, target)

    def check_versions(self):
        return subprocess.run(
            [sys.executable, str(self.root / ".github/scripts/check-versions.py")],
            capture_output=True, text=True, check=False,
        )

    def test_example_package_can_have_its_own_version(self):
        result = self.check_versions()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_stale_example_dependency_is_rejected(self):
        source = EXAMPLE + "Cargo.toml"
        path = self.root / source
        path.write_text(re.sub(r'^vitri = "[^"]+"', 'vitri = "0.0.0"',
                               path.read_text(), count=1, flags=re.M))
        result = self.check_versions()
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertIn(source, result.stdout)

    def test_stale_example_lockfile_is_rejected(self):
        source = EXAMPLE + "Cargo.lock"
        path = self.root / source
        path.write_text(re.sub(r'(name = "vitri"\nversion = ")[^"]+',
                               r'\g<1>0.0.0', path.read_text(), count=1))
        result = self.check_versions()
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertIn(source, result.stdout)


if __name__ == "__main__":
    unittest.main()
