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
run_tests() {
  local device_id="$1" bundle="$2"
  shift 2
  set +e
  xcodebuild test -project ios/PinVoyager.xcodeproj -scheme PinVoyager \
    -testPlan PinVoyager -destination "platform=iOS Simulator,id=$device_id" \
    -derivedDataPath ios/.derivedData -enableCodeCoverage YES \
    -resultBundlePath "$bundle" CODE_SIGN_IDENTITY=- "$@"
  local result=$?
  set -e
  local profile="ios/.derivedData/Build/ProfileData/$device_id/Coverage.profdata"
  if [[ -f "$profile" ]]; then
    cp "$profile" "${bundle%.xcresult}.profdata"
  fi
  return "$result"
}
# Seeding requires a booted simulator. Reuse the shared image fixture.
if ! xcrun simctl list devices booted --json | python3 -c 'import json,sys; ids=[d["udid"] for ds in json.load(sys.stdin)["devices"].values() for d in ds]; sys.exit(sys.argv[1] not in ids)' "$phone_id"; then
  xcrun simctl boot "$phone_id"
fi
xcrun simctl bootstatus "$phone_id" -b
xcrun simctl addmedia "$phone_id" crates/api/tests/fixtures/quadrants-rgb.heic
run_tests "$phone_id" "$result_path"
run_tests "$ipad_id" "$ipad_result_path" \
  -only-testing:PinVoyagerUITests/NavigationTests/testGuestTabsAndExplicitSignIn
python3 ios/Scripts/coverage.py "$result_path" --derived-data ios/.derivedData \
  --profile "${result_path%.xcresult}.profdata" \
  --profile "${ipad_result_path%.xcresult}.profdata"
