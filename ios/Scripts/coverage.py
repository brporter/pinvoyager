#!/usr/bin/env python3
"""Measure all production Swift; fail if any file is missing or coverage is below 95%."""

import argparse
import json
from pathlib import Path
import subprocess
import sys
import tempfile


def run_json(*arguments):
    return json.loads(subprocess.check_output(arguments, text=True))


def collect_profiles(derived_data, devices, profiles, directories):
    snapshots = []
    for directory in directories:
        workers = sorted(directory.glob("*/Coverage.profdata"))
        if not workers:
            raise ValueError(f"No simulator coverage profiles in snapshot: {directory}")
        snapshots.extend(workers)
    return list(dict.fromkeys(
        [derived_data / "Build/ProfileData" / device / "Coverage.profdata" for device in devices]
        + profiles
        + snapshots
    ))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("result_bundle", type=Path)
    parser.add_argument("--derived-data", type=Path, required=True)
    parser.add_argument("--device", action="append", default=[])
    parser.add_argument("--profile", type=Path, action="append", default=[])
    parser.add_argument("--profile-directory", type=Path, action="append", default=[])
    parser.add_argument("--report-only", action="store_true",
                        help="Report reduced-suite coverage without enforcing the full-suite 95% threshold.")
    args = parser.parse_args()
    try:
        profiles = collect_profiles(args.derived_data, args.device, args.profile, args.profile_directory)
    except ValueError as error:
        parser.error(str(error))
    if not profiles:
        parser.error("Provide at least one --device, --profile, or --profile-directory.")
    for profile in profiles:
        if not profile.is_file():
            parser.error(f"Missing coverage profile: {profile}")
    root = Path(__file__).resolve().parents[1]
    sources = {file.resolve() for directory in (root / "Sources", root / "PinVoyager") for file in directory.rglob("*.swift")}
    if not sources:
        print("No production Swift files found.", file=sys.stderr)
        return 1

    # Retain Xcode's report. Some Xcode versions omit Swift package files from xccov,
    # even though their binaries have instrumentation and their counters were recorded.
    report = run_json("xcrun", "xccov", "view", "--report", "--json", str(args.result_bundle))
    output = args.result_bundle.parent / (args.result_bundle.stem + "-coverage.json")
    output.write_text(json.dumps(report, indent=2) + "\n")
    products = args.derived_data / "Build/Products/Debug-iphonesimulator"
    app = products / "PinVoyager.app"
    binary = app / "PinVoyager.debug.dylib"
    if not binary.exists():
        binary = app / "PinVoyager"
    binaries = [binary]
    for framework in (app / "Frameworks").glob("*.framework"):
        candidate = framework / framework.stem
        if candidate.exists():
            binaries.append(candidate)
    with tempfile.TemporaryDirectory(prefix="pinvoyager-coverage-") as directory:
        profile = profiles[0]
        if len(profiles) > 1:
            profile = Path(directory) / "combined.profdata"
            subprocess.check_call(["xcrun", "llvm-profdata", "merge", "--sparse",
                                   *map(str, profiles), "-o", str(profile)])
        command = ["xcrun", "llvm-cov", "export", str(binaries[0]), "-instr-profile", str(profile)]
        for candidate in binaries[1:]:
            command.extend(["-object", str(candidate)])
        coverage = run_json(*command)
    measured = {}
    for item in coverage["data"]:
        for file in item["files"]:
            path = Path(file["filename"]).resolve()
            if path in sources:
                measured[path] = file["summary"]["lines"]
    missing = sources - measured.keys()
    if missing:
        for path in sorted(missing):
            print(f"Missing production instrumentation: {path.relative_to(root)}", file=sys.stderr)
        return 1
    covered = sum(value["covered"] for value in measured.values())
    count = sum(value["count"] for value in measured.values())
    percentage = 100 * covered / count if count else 0
    summary = {"covered_lines": covered, "executable_lines": count, "percentage": percentage, "required_percentage": 95, "threshold_enforced": not args.report_only, "profiles": [str(profile) for profile in profiles], "files": {str(path.relative_to(root)): value for path, value in sorted(measured.items())}}
    output.with_name(output.stem + "-swift.json").write_text(json.dumps(summary, indent=2) + "\n")
    requirement = "report only; full local suite requires 95%" if args.report_only else "required 95%"
    print(f"Production Swift executable lines: {covered}/{count} ({percentage:.2f}%), {requirement}")
    return 0 if args.report_only or percentage >= 95 else 1


if __name__ == "__main__":
    sys.exit(main())
