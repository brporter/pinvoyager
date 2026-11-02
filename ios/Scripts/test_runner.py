"""Check that the CI mode runs non-UI tests without starting system-picker journeys."""

import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest


class TestRunnerTests(unittest.TestCase):
    def test_non_ui_mode_skips_ui_target_and_reports_only_current_coverage(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            scripts = root / "ios/Scripts"
            scripts.mkdir(parents=True)
            shutil.copy(Path(__file__).with_name("test.sh"), scripts / "test.sh")
            (scripts / "coverage.py").write_text(
                "import json,sys\nfrom pathlib import Path\n"
                "Path('coverage-args.json').write_text(json.dumps(sys.argv[1:]))\n"
            )
            tools = root / "tools"
            tools.mkdir()
            (tools / "python3").symlink_to(sys.executable)
            developer = root / "developer"
            (developer / "usr/bin").mkdir(parents=True)
            builder = developer / "usr/bin/xcodebuild"
            builder.write_text(
                "#!/bin/bash\n"
                "printf '%s\\n' \"$@\" >> build-args.txt\n"
                "mkdir -p ios/TestResults ios/.derivedData/Build/ProfileData/current\n"
                "touch ios/.derivedData/Build/ProfileData/current/Coverage.profdata\n"
            )
            builder.chmod(0o755)
            # Explicit phone selection must avoid simctl, iPad discovery, cloning, and media seeding.
            xcrun = tools / "xcrun"
            xcrun.write_text("#!/bin/bash\necho 'Unexpected simctl command' >&2\nexit 42\n")
            xcrun.chmod(0o755)
            stale = root / "ios/.derivedData/Build/ProfileData/stale/Coverage.profdata"
            stale.parent.mkdir(parents=True)
            stale.touch()
            result = subprocess.run(
                ["bash", str(scripts / "test.sh"), "phone"], cwd=root,
                env={**os.environ, "PATH": f"{tools}:{developer}/usr/bin:{os.environ['PATH']}",
                     "DEVELOPER_DIR": str(developer), "IOS_UI_TESTS": "0"},
                capture_output=True, text=True,
            )
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            arguments = (root / "build-args.txt").read_text().splitlines()
            self.assertIn("-skip-testing:PinVoyagerUITests", arguments)
            self.assertEqual(arguments.count("test"), 1)
            self.assertFalse(stale.exists())
            coverage = json.loads((root / "coverage-args.json").read_text())
            self.assertIn("--report-only", coverage)
            self.assertEqual(coverage.count("--profile-directory"), 1)
            snapshot = root / coverage[coverage.index("--profile-directory") + 1]
            self.assertTrue((snapshot / "current/Coverage.profdata").is_file())
            self.assertFalse((snapshot / "stale").exists())


if __name__ == "__main__":
    unittest.main()
