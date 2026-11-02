"""Check that parallel test runs cannot silently lose or reuse coverage counters."""

from pathlib import Path
import tempfile
import unittest

from coverage import collect_profiles


class CoverageProfilesTests(unittest.TestCase):
    def test_collects_every_worker_and_ipad_without_reusing_stale_derived_data(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            derived = root / "derived"
            stale = derived / "Build/ProfileData/old-device/Coverage.profdata"
            stale.parent.mkdir(parents=True)
            stale.touch()
            expected = []
            for run, devices in [("phone", ["worker-1", "worker-2"]), ("ipad", ["tablet"])]:
                for device in devices:
                    profile = root / run / device / "Coverage.profdata"
                    profile.parent.mkdir(parents=True)
                    profile.touch()
                    expected.append(profile)
            actual = collect_profiles(derived, [], [], [root / "phone", root / "ipad"])
            self.assertEqual(actual, expected)
            self.assertNotIn(stale, actual)

    def test_missing_or_empty_worker_snapshots_fail_instead_of_using_partial_coverage(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for snapshot in [root, root / "missing"]:
                with self.subTest(snapshot=snapshot), self.assertRaises(ValueError):
                    collect_profiles(root, [], [], [snapshot])

    def test_existing_device_and_explicit_profile_arguments_still_deduplicate(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            profile = root / "Build/ProfileData/phone/Coverage.profdata"
            profile.parent.mkdir(parents=True)
            profile.touch()
            self.assertEqual(collect_profiles(root, ["phone"], [profile], []), [profile])


if __name__ == "__main__":
    unittest.main()
