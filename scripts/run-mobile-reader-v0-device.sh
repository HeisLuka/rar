#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ANDROID_APP="$ROOT/apps/chaptera-mobile-android"
JNI_OUT="$ANDROID_APP/app/src/main/jniLibs"
ASSET_DIR="$ANDROID_APP/app/src/androidTest/assets"
RECEIPT_DIR="$ROOT/artifacts/mobile-reader-v0-device"
mkdir -p "$RECEIPT_DIR" "$ASSET_DIR"

for tool in adb cargo cargo-ndk gradle curl git python3; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    echo "$tool is required" >&2
    exit 2
  fi
done

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
AIRPLANE_MODE="$(adb -s "$SERIAL" shell settings get global airplane_mode_on 2>/dev/null | tr -d '\r' || true)"

if [ "$AIRPLANE_MODE" != "1" ]; then
  echo "Physical V0 acceptance requires airplane mode to be enabled on the device before the run." >&2
  echo "Observed airplane_mode_on=$AIRPLANE_MODE" >&2
  exit 6
fi

case "$ABI" in
  arm64-v8a) NDK_TARGET="arm64-v8a" ;;
  x86_64) NDK_TARGET="x86_64" ;;
  *)
    echo "Unsupported test-device ABI: $ABI" >&2
    exit 4
    ;;
esac

echo "Device: $MODEL / API $API / ABI $ABI"
echo "Preparing local Reader JNI for $NDK_TARGET"

rm -rf "$JNI_OUT"
cargo +1.94.1 ndk \
  -t "$NDK_TARGET" \
  -o "$JNI_OUT" \
  build \
  --manifest-path "$ROOT/crates/chaptera-mobile-reader-jni/Cargo.toml" \
  --release

download_fixture() {
  local name="$1"
  local blob_sha="$2"
  local file="$ASSET_DIR/$name"
  if [ ! -f "$file" ]; then
    curl --fail --location \
      "https://raw.githubusercontent.com/apache/poi/942d95d85b15d0dfdb3bc9ba1b4f273f277757c8/test-data/publisher/$name" \
      --output "$file"
  fi
  local actual_blob_sha
  actual_blob_sha="$(git hash-object "$file")"
  if [ "$actual_blob_sha" != "$blob_sha" ]; then
    echo "Fixture blob mismatch for $name: $actual_blob_sha != $blob_sha" >&2
    exit 5
  fi
}

download_fixture "Simple.pub" "2397b9d01cfa159de2f73cf5c990d9698eb69567"
download_fixture "SampleBrochure.pub" "00deec14dc6cff6d8a47b120d6d84e7e20c72166"
download_fixture "SampleNewsletter.pub" "94900925af5832c493784f3cb51563f838a64df8"

echo "Building app and instrumentation APK"
gradle -p "$ANDROID_APP" --no-daemon :app:assembleDebug :app:assembleDebugAndroidTest

echo "Disabling radios where the device permits it"
adb -s "$SERIAL" shell svc wifi disable || true
adb -s "$SERIAL" shell svc data disable || true

START_UTC="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
set +e
gradle -p "$ANDROID_APP" --no-daemon :app:connectedDebugAndroidTest \
  -Pandroid.testInstrumentationRunnerArguments.class=com.chaptera.reader.V0UserCycleInstrumentedTest
TEST_RC=$?

PERF_RC=0
if [ "$TEST_RC" -eq 0 ]; then
  gradle -p "$ANDROID_APP" --no-daemon :app:connectedDebugAndroidTest \
    -Pandroid.testInstrumentationRunnerArguments.class=com.chaptera.reader.PhysicalPerfInstrumentedTest
  PERF_RC=$?
fi
set -e
END_UTC="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

WIFI_STATE="$(adb -s "$SERIAL" shell dumpsys wifi 2>/dev/null | grep -m1 -E 'Wi-Fi is|wifi state' | tr -d '\r' || true)"
DATA_STATE="$(adb -s "$SERIAL" shell dumpsys telephony.registry 2>/dev/null | grep -m1 -E 'mDataConnectionState|mDataConnectionNetworkType' | tr -d '\r' || true)"

STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
RECEIPT="$RECEIPT_DIR/receipt-$STAMP.json"
PERF_RECEIPT="$RECEIPT_DIR/perf-$STAMP.json"

if [ "$TEST_RC" -eq 0 ] && [ "$PERF_RC" -eq 0 ]; then
  if adb -s "$SERIAL" exec-out run-as com.chaptera.reader cat files/chaptera-mobile-perf.json > "$PERF_RECEIPT"; then
    echo "Performance receipt: $PERF_RECEIPT"
  else
    echo "Could not pull performance receipt" >&2
    PERF_RC=1
  fi
fi

python3 - "$RECEIPT" "$MODEL" "$API" "$ABI" "$ANDROID_ID" "$AIRPLANE_MODE" "$START_UTC" "$END_UTC" "$TEST_RC" "$PERF_RC" "$WIFI_STATE" "$DATA_STATE" <<'PY'
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
        "airplane_mode_enabled": sys.argv[6] == "1",
    },
    "started_at_utc": sys.argv[7],
    "finished_at_utc": sys.argv[8],
    "v0_user_cycle_exit_code": int(sys.argv[9]),
    "physical_perf_exit_code": int(sys.argv[10]),
    "wifi_observation": sys.argv[11],
    "mobile_data_observation": sys.argv[12],
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
