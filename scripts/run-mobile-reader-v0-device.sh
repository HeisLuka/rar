#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ANDROID_APP="$ROOT/apps/chaptera-mobile-android"
JNI_OUT="$ANDROID_APP/app/src/main/jniLibs"
ASSET_DIR="$ANDROID_APP/app/src/androidTest/assets"
RECEIPT_DIR="$ROOT/artifacts/mobile-reader-v0-device"
mkdir -p "$RECEIPT_DIR" "$ASSET_DIR"

if ! command -v adb >/dev/null 2>&1; then
  echo "adb is required" >&2
  exit 2
fi
if ! command -v cargo >/dev/null 2>&1; then
  echo "cargo is required" >&2
  exit 2
fi
if ! command -v cargo-ndk >/dev/null 2>&1; then
  echo "cargo-ndk is required: cargo install cargo-ndk --locked" >&2
  exit 2
fi
if ! command -v gradle >/dev/null 2>&1; then
  echo "gradle is required" >&2
  exit 2
fi

mapfile -t DEVICES < <(adb devices | awk 'NR>1 && $2=="device" {print $1}')
if [ "${#DEVICES[@]}" -ne 1 ]; then
  echo "Expected exactly one authorized Android device; found ${#DEVICES[@]}" >&2
  adb devices -l >&2
  exit 3
fi
SERIAL="${DEVICES[0]}"

MODEL="$(adb -s "$SERIAL" shell getprop ro.product.model | tr -d '\r')"
API="$(adb -s "$SERIAL" shell getprop ro.build.version.sdk | tr -d '\r')"
ABI="$(adb -s "$SERIAL" shell getprop ro.product.cpu.abi | tr -d '\r')"
ANDROID_ID="$(adb -s "$SERIAL" shell settings get secure android_id 2>/dev/null | tr -d '\r' || true)"

case "$ABI" in
  arm64-v8a)
    NDK_TARGET="arm64-v8a"
    ;;
  x86_64)
    NDK_TARGET="x86_64"
    ;;
  *)
    echo "Unsupported test-device ABI: $ABI" >&2
    exit 4
    ;;
esac

echo "Device: $MODEL / API $API / ABI $ABI"
echo "Preparing local Reader JNI for $NDK_TARGET"

rm -rf "$JNI_OUT"
cargo +1.94.1 ndk   -t "$NDK_TARGET"   -o "$JNI_OUT"   build   --manifest-path "$ROOT/crates/chaptera-mobile-reader-jni/Cargo.toml"   --release

FIXTURE="$ASSET_DIR/SampleNewsletter.pub"
EXPECTED_SHA="6a825ba26ba35d6e885acdc62e859591ed37cb0ff7480b554b9cb362b644dfcf"
if [ ! -f "$FIXTURE" ]; then
  curl --fail --location     "https://raw.githubusercontent.com/apache/poi/942d95d85b15d0dfdb3bc9ba1b4f273f277757c8/test-data/publisher/SampleNewsletter.pub"     --output "$FIXTURE"
fi
printf '%s  %s\n' "$EXPECTED_SHA" "$FIXTURE" | sha256sum --check

echo "Building app and instrumentation APK"
gradle -p "$ANDROID_APP" --no-daemon :app:assembleDebug :app:assembleDebugAndroidTest

echo "Disabling radios where the device permits it"
adb -s "$SERIAL" shell svc wifi disable || true
adb -s "$SERIAL" shell svc data disable || true

START_UTC="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
set +e
gradle -p "$ANDROID_APP" --no-daemon :app:connectedDebugAndroidTest   -Pandroid.testInstrumentationRunnerArguments.class=com.chaptera.reader.V0UserCycleInstrumentedTest
TEST_RC=$?
set -e
END_UTC="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

WIFI_STATE="$(adb -s "$SERIAL" shell dumpsys wifi 2>/dev/null | grep -m1 -E 'Wi-Fi is|wifi state' | tr -d '\r' || true)"
DATA_STATE="$(adb -s "$SERIAL" shell dumpsys telephony.registry 2>/dev/null | grep -m1 -E 'mDataConnectionState|mDataConnectionNetworkType' | tr -d '\r' || true)"

RECEIPT="$RECEIPT_DIR/receipt-$(date -u +%Y%m%dT%H%M%SZ).json"
python3 - "$RECEIPT" "$MODEL" "$API" "$ABI" "$ANDROID_ID" "$START_UTC" "$END_UTC" "$TEST_RC" "$PERF_RC" "$WIFI_STATE" "$DATA_STATE" <<'PY'
import json
import sys
from pathlib import Path

receipt = {
    "schema": "chaptera.mobile-reader-v0.device-receipt.v1",
    "device": {
        "model": sys.argv[2],
        "api": sys.argv[3],
        "abi": sys.argv[4],
        "android_id_present": bool(sys.argv[5] and sys.argv[5] != "null"),
    },
    "started_at_utc": sys.argv[6],
    "finished_at_utc": sys.argv[7],
    "v0_user_cycle_exit_code": int(sys.argv[8]),
    "physical_perf_exit_code": int(sys.argv[9]),
    "wifi_observation": sys.argv[10],
    "mobile_data_observation": sys.argv[11],
    "contains_document_bytes": False,
    "contains_recovered_document_text": False,
}
Path(sys.argv[1]).write_text(json.dumps(receipt, indent=2) + "\n", encoding="utf-8")
PY

echo "Receipt: $RECEIPT"
if [ "$TEST_RC" -ne 0 ]; then
  echo "Physical-device V0 acceptance failed" >&2
  exit "$TEST_RC"
fi
if [ "$PERF_RC" -ne 0 ]; then
  echo "Physical-device performance receipt failed" >&2
  exit "$PERF_RC"
fi

echo "Physical-device V0 acceptance and performance receipt passed"
