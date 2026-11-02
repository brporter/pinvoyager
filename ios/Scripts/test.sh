#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/../.."
export DEVELOPER_DIR="${DEVELOPER_DIR:-$(xcode-select -p)}"
if [[ ! -x "$DEVELOPER_DIR/usr/bin/xcodebuild" ]]; then
  echo "Select a full Xcode installation with DEVELOPER_DIR before running iOS tests." >&2
  exit 1
fi
choose_device() {
  xcrun simctl list devices available --json | python3 -c 'import json,sys; devices=json.load(sys.stdin)["devices"]; print(next(device["udid"] for runtime, items in devices.items() if "iOS" in runtime for device in items if device["isAvailable"] and device["name"].startswith(sys.argv[1])))' "$1"
}
phone_id="${1:-$(choose_device iPhone)}"
ipad_id="${2:-$(choose_device iPad)}"
result_path="ios/TestResults/$(date +%Y%m%d-%H%M%S).xcresult"
ipad_result_path="${result_path%.xcresult}-ipad.xcresult"
workers="${IOS_TEST_WORKERS:-2}"
if [[ "$workers" != 1 && "$workers" != 2 ]]; then
  echo "IOS_TEST_WORKERS must be 1 or 2." >&2
  exit 1
fi
run_tests() {
  local device_id="$1" bundle="$2"
  shift 2
  # Xcode may emit separate profiles for simulator workers. Remove earlier counters so a
  # later run cannot pass coverage using tests that it did not execute.
  local profile_root="ios/.derivedData/Build/ProfileData"
  rm -rf "$profile_root"
  set +e
  xcodebuild test -project ios/PinVoyager.xcodeproj -scheme PinVoyager \
    -testPlan PinVoyager -destination "platform=iOS Simulator,id=$device_id" \
    -derivedDataPath ios/.derivedData -enableCodeCoverage YES \
    -resultBundlePath "$bundle" CODE_SIGN_IDENTITY=- "$@"
  local result=$?
  set -e
  if [[ -d "$profile_root" ]]; then
    cp -R "$profile_root" "${bundle%.xcresult}-profiles"
  fi
  return "$result"
}
# Use a disposable, erased clone so system notification authorization starts undetermined.
# Preserve the caller's simulator and clean up even when xcodebuild fails.
phone_id="$(xcrun simctl clone "$phone_id" "PinVoyager tests $$")"
cleanup_phone() {
  xcrun simctl shutdown "$phone_id" >/dev/null 2>&1 || true
  xcrun simctl delete "$phone_id"
}
trap cleanup_phone EXIT
xcrun simctl erase "$phone_id"
# Seeding requires a booted simulator. Reuse the shared image fixture.
if ! xcrun simctl list devices booted --json | python3 -c 'import json,sys; ids=[d["udid"] for ds in json.load(sys.stdin)["devices"].values() for d in ds]; sys.exit(sys.argv[1] not in ids)' "$phone_id"; then
  xcrun simctl boot "$phone_id"
fi
xcrun simctl bootstatus "$phone_id" -b
xcrun simctl addmedia "$phone_id" crates/api/tests/fixtures/quadrants-rgb.heic
run_tests "$phone_id" "$result_path" \
  -parallel-testing-enabled YES -parallel-testing-worker-count "$workers"
run_tests "$ipad_id" "$ipad_result_path" \
  -parallel-testing-enabled NO \
  -only-testing:PinVoyagerUITests/NavigationTests/testGuestTabsAndExplicitSignIn
python3 ios/Scripts/coverage.py "$result_path" --derived-data ios/.derivedData \
  --profile-directory "${result_path%.xcresult}-profiles" \
  --profile-directory "${ipad_result_path%.xcresult}-profiles"
