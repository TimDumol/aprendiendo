#!/usr/bin/env bash
# Build and install the standalone Android practice app on a connected ADB target.
set -Eeuo pipefail

ROOT="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
APP_DIR="$ROOT/apps/practice"
PACKAGE_NAME="${PRACTICE_ANDROID_PACKAGE:-com.timdumol.aprendiendo.practice}"
VARIANT="${PRACTICE_ANDROID_VARIANT:-release}"
API_URL="${EXPO_PUBLIC_PRACTICE_API_URL:-https://mars.timdumol.com}"
OAUTH_ISSUER="${EXPO_PUBLIC_OAUTH_ISSUER:-}"
OAUTH_AUTHORIZATION_URL="${EXPO_PUBLIC_OAUTH_AUTHORIZATION_URL:-}"
OAUTH_TOKEN_URL="${EXPO_PUBLIC_OAUTH_TOKEN_URL:-}"
OAUTH_CLIENT_ID="${EXPO_PUBLIC_OAUTH_CLIENT_ID:-}"
OAUTH_SCOPE="${EXPO_PUBLIC_OAUTH_SCOPE:-}"
OAUTH_REDIRECT_URI="${EXPO_PUBLIC_OAUTH_REDIRECT_URI:-}"
OAUTH_RESOURCE="${EXPO_PUBLIC_OAUTH_RESOURCE:-}"
REQUESTED_DEVICE="${PRACTICE_ANDROID_DEVICE:-${ANDROID_SERIAL:-}}"
ADB_OVERRIDE="${PRACTICE_ADB:-}"
PHONE_ONLY=0
FORCE_BUILD="${PRACTICE_ANDROID_BUILD:-0}"
KEEP_NATIVE="${PRACTICE_ANDROID_KEEP_NATIVE:-0}"
APK_PATH="${PRACTICE_ANDROID_APK:-}"

usage() {
  printf '%s\n' \
    'Usage: ./scripts/build-install-practice-android.sh [options]' \
    '' \
    'Builds the standalone APK when missing (or when --build is supplied),' \
    'installs it on one connected Android target, and launches the app.' \
    '' \
    'Options:' \
    '  --phone                 Ignore emulators when selecting a target' \
    '  --build                 Always rebuild the APK' \
    '  --debug                 Build and install the debug variant' \
    '  --release               Build and install the release variant (default)' \
    '  --apk PATH              APK path to build/use' \
    '  --device SERIAL         ADB serial to install on' \
    '  --api-url URL           Practice API URL bundled into the app' \
    '  --keep-native           Keep the generated Android project after the build' \
    '  -h, --help              Show this help' \
    '' \
    'Environment:' \
    '  PRACTICE_ANDROID_APK       APK path override' \
    '  PRACTICE_ANDROID_BUILD=1   Always rebuild the APK' \
    '  PRACTICE_ANDROID_DEVICE   ADB serial when more than one target is connected' \
    '  ANDROID_SERIAL             Standard ADB serial override' \
    '  PRACTICE_ADB               ADB executable override' \
    '  PRACTICE_GRADLE_MAX_WORKERS=1     Gradle worker limit' \
    '  PRACTICE_NATIVE_COMPILE_JOBS=1    CMake/Ninja compile limit' \
    '  PRACTICE_GRADLE_HEAP=2048m        Gradle maximum heap' \
    '  PRACTICE_GRADLE_METASPACE=1024m   Gradle maximum metaspace' \
    '  PRACTICE_ANDROID_ARCHITECTURES    Comma-separated ABI override' \
    '  .phonerc                    Local fallback containing PRACTICE_ANDROID_DEVICE'
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --phone)
      PHONE_ONLY=1
      ;;
    --build)
      FORCE_BUILD=1
      ;;
    --debug)
      VARIANT=debug
      ;;
    --release)
      VARIANT=release
      ;;
    --apk)
      [[ $# -ge 2 ]] || { usage >&2; exit 2; }
      APK_PATH="$2"
      shift
      ;;
    --device)
      [[ $# -ge 2 ]] || { usage >&2; exit 2; }
      REQUESTED_DEVICE="$2"
      shift
      ;;
    --api-url)
      [[ $# -ge 2 ]] || { usage >&2; exit 2; }
      API_URL="$2"
      shift
      ;;
    --keep-native)
      KEEP_NATIVE=1
      ;;
    --help|-h)
      usage
      exit 0
      ;;
    *)
      usage >&2
      exit 2
      ;;
  esac
  shift
done

case "$VARIANT" in
  debug|release)
    ;;
  *)
    printf 'PRACTICE_ANDROID_VARIANT must be debug or release (got %s).\n' "$VARIANT" >&2
    exit 2
    ;;
esac

gradle_workers="${PRACTICE_GRADLE_MAX_WORKERS:-1}"
native_compile_jobs="${PRACTICE_NATIVE_COMPILE_JOBS:-1}"
gradle_heap="${PRACTICE_GRADLE_HEAP:-2048m}"
gradle_metaspace="${PRACTICE_GRADLE_METASPACE:-1024m}"
for value_name in gradle_workers native_compile_jobs; do
  value="${!value_name}"
  if [[ ! "$value" =~ ^[1-9][0-9]*$ ]]; then
    printf '%s must be a positive integer.\n' "$value_name" >&2
    exit 2
  fi
done
for value_name in gradle_heap gradle_metaspace; do
  value="${!value_name}"
  if [[ ! "$value" =~ ^[1-9][0-9]*[mMgG]$ ]]; then
    printf '%s must be a memory value such as 2048m.\n' "$value_name" >&2
    exit 2
  fi
done

if [[ -z "$APK_PATH" ]]; then
  APK_PATH="$APP_DIR/android/app/build/outputs/apk/$VARIANT/app-$VARIANT.apk"
elif [[ "$APK_PATH" != /* ]]; then
  APK_PATH="$ROOT/$APK_PATH"
fi

SDK_DIR="${ANDROID_SDK_ROOT:-${ANDROID_HOME:-}}"
if [[ -z "$SDK_DIR" ]]; then
  for candidate in "$HOME/Android/Sdk" /usr/lib/android-sdk; do
    if [[ -d "$candidate" ]]; then
      SDK_DIR="$candidate"
      break
    fi
  done
fi
if [[ -n "$SDK_DIR" ]]; then
  export ANDROID_HOME="$SDK_DIR"
  export ANDROID_SDK_ROOT="$SDK_DIR"
fi

ADB_BIN="$ADB_OVERRIDE"
if [[ -z "$ADB_BIN" && -n "$SDK_DIR" && -x "$SDK_DIR/platform-tools/adb" ]]; then
  ADB_BIN="$SDK_DIR/platform-tools/adb"
fi
if [[ -z "$ADB_BIN" ]]; then
  ADB_BIN="$(command -v adb || true)"
fi
if [[ -z "$ADB_BIN" ]]; then
  printf '%s\n' 'adb is required. Install Android platform-tools or set PRACTICE_ADB.' >&2
  exit 1
fi
for command in java node npx; do
  command -v "$command" >/dev/null || {
    printf 'missing required command: %s\n' "$command" >&2
    exit 127
  }
done
[[ -d "$APP_DIR" ]] || {
  printf 'missing practice app: %s\n' "$APP_DIR" >&2
  exit 2
}

is_emulator() {
  local serial="$1"
  local qemu
  [[ "$serial" == emulator-* ]] && return 0
  qemu="$("$ADB_BIN" -s "$serial" shell getprop ro.kernel.qemu 2>/dev/null | tr -d '\r' || true)"
  [[ "$qemu" == 1 ]]
}

"$ADB_BIN" start-server >/dev/null
configured_device=""
phonerc_path="$ROOT/.phonerc"
if [[ -z "$REQUESTED_DEVICE" && -f "$phonerc_path" ]]; then
  configured_device="$(sed -nE \
    's/^[[:space:]]*PRACTICE_ANDROID_DEVICE[[:space:]]*=[[:space:]]*([^[:space:]#]+).*/\1/p' \
    "$phonerc_path" | tail -n 1)"
  REQUESTED_DEVICE="$configured_device"
fi

target_device="$REQUESTED_DEVICE"
if [[ -z "$target_device" ]]; then
  mapfile -t connected_devices < <("$ADB_BIN" devices | awk 'NR > 1 && $2 == "device" { print $1 }')
  if [[ "$PHONE_ONLY" == 1 ]]; then
    phone_devices=()
    for device in "${connected_devices[@]}"; do
      if ! is_emulator "$device"; then
        phone_devices+=("$device")
      fi
    done
    connected_devices=("${phone_devices[@]}")
  fi
  case "${#connected_devices[@]}" in
    0)
      if [[ "$PHONE_ONLY" == 1 ]]; then
        printf '%s\n' 'No physical Android device is connected.' >&2
      else
        printf '%s\n' 'No Android device or emulator is connected.' >&2
      fi
      printf '%s\n' 'Connect a target, authorize it, or set PRACTICE_ANDROID_DEVICE.' >&2
      exit 1
      ;;
    1)
      target_device="${connected_devices[0]}"
      ;;
    *)
      printf '%s\n' 'More than one Android target is connected; set PRACTICE_ANDROID_DEVICE.' >&2
      printf 'Connected targets: %s\n' "${connected_devices[*]}" >&2
      exit 1
      ;;
  esac
fi

device_state="$("$ADB_BIN" -s "$target_device" get-state 2>/dev/null || true)"
if [[ "$device_state" != device ]]; then
  printf 'Android target %s is not ready (state: %s).\n' "$target_device" "${device_state:-unknown}" >&2
  exit 1
fi

target_is_emulator=0
if is_emulator "$target_device"; then
  target_is_emulator=1
fi
if [[ "$PHONE_ONLY" == 1 && "$target_is_emulator" == 1 ]]; then
  printf 'Android target %s is an emulator, but --phone was specified.\n' "$target_device" >&2
  exit 1
fi

architectures="${PRACTICE_ANDROID_ARCHITECTURES:-}"
if [[ -z "$architectures" ]]; then
  if [[ "$target_is_emulator" == 1 ]]; then
    abi_list="$("$ADB_BIN" -s "$target_device" shell getprop ro.product.cpu.abilist 2>/dev/null | tr -d '\r' || true)"
    case "$abi_list" in
      *x86_64*) architectures=x86_64 ;;
      *x86*) architectures=x86 ;;
      *arm64-v8a*) architectures=arm64-v8a ;;
      *) architectures=arm64-v8a ;;
    esac
  else
    architectures=arm64-v8a
  fi
fi
if [[ ! "$architectures" =~ ^[a-z0-9-]+(,[a-z0-9-]+)*$ ]]; then
  printf 'PRACTICE_ANDROID_ARCHITECTURES is invalid: %s\n' "$architectures" >&2
  exit 2
fi

if [[ "$API_URL" == "https://mars.timdumol.com" ]]; then
  OAUTH_ISSUER="${OAUTH_ISSUER:-https://auth.aries.timdumol.com}"
  OAUTH_AUTHORIZATION_URL="${OAUTH_AUTHORIZATION_URL:-$OAUTH_ISSUER/authorize}"
  OAUTH_TOKEN_URL="${OAUTH_TOKEN_URL:-$OAUTH_ISSUER/api/oidc/token}"
  OAUTH_CLIENT_ID="${OAUTH_CLIENT_ID:-aprendiendo-practice-mobile}"
  OAUTH_SCOPE="${OAUTH_SCOPE:-openid profile email learning:access}"
  OAUTH_REDIRECT_URI="${OAUTH_REDIRECT_URI:-aprendiendo-practice-mvp://oauth/callback}"
  OAUTH_RESOURCE="${OAUTH_RESOURCE:-$API_URL}"
fi

build_date="${EXPO_PUBLIC_PRACTICE_BUILD_DATE:-}"
if [[ -z "$build_date" ]]; then
  build_date="$(date -u '+%Y-%m-%dT%H:%M:%SZ')"
fi

export EXPO_PUBLIC_PRACTICE_API_URL="$API_URL"
export EXPO_PUBLIC_OAUTH_ISSUER="$OAUTH_ISSUER"
export EXPO_PUBLIC_OAUTH_AUTHORIZATION_URL="$OAUTH_AUTHORIZATION_URL"
export EXPO_PUBLIC_OAUTH_TOKEN_URL="$OAUTH_TOKEN_URL"
export EXPO_PUBLIC_OAUTH_CLIENT_ID="$OAUTH_CLIENT_ID"
export EXPO_PUBLIC_OAUTH_SCOPE="$OAUTH_SCOPE"
export EXPO_PUBLIC_OAUTH_REDIRECT_URI="$OAUTH_REDIRECT_URI"
export EXPO_PUBLIC_OAUTH_RESOURCE="$OAUTH_RESOURCE"
export EXPO_PUBLIC_PRACTICE_BUILD_DATE="$build_date"
export EXPO_PUBLIC_PRACTICE_BUILD_VARIANT="$VARIANT"
export PRACTICE_ANDROID_ARCHITECTURES="$architectures"
export PRACTICE_GRADLE_HEAP="$gradle_heap"
export PRACTICE_GRADLE_METASPACE="$gradle_metaspace"
export PRACTICE_NATIVE_COMPILE_JOBS="$native_compile_jobs"
export PRACTICE_GRADLE_MAX_WORKERS="$gradle_workers"
export CMAKE_BUILD_PARALLEL_LEVEL="$native_compile_jobs"
export NINJAFLAGS="-j$native_compile_jobs"
unset NO_COLOR FORCE_COLOR
if [[ "$VARIANT" == release ]]; then
  export NODE_ENV=production
fi

native_project_created=false
if [[ ! -d "$APP_DIR/android" ]]; then
  native_project_created=true
fi
cleanup() {
  local exit_code=$?
  if [[ "$native_project_created" == true && "$KEEP_NATIVE" != 1 ]]; then
    rm -rf -- "$APP_DIR/android"
  fi
  exit "$exit_code"
}
trap cleanup EXIT

if [[ ! -f "$APK_PATH" || "$FORCE_BUILD" == 1 ]]; then
  echo "Generating Android native project"
  (cd "$APP_DIR" && npx expo prebuild --platform android --no-install)

  gradle_task="assemble${VARIANT^}"
  echo "Building $gradle_task for $architectures (Gradle workers: $gradle_workers, native jobs: $native_compile_jobs)"
  (cd "$APP_DIR/android" && ./gradlew --no-daemon --max-workers="$gradle_workers" \
    -Dorg.gradle.workers.max="$gradle_workers" ":app:$gradle_task")
fi

if [[ ! -f "$APK_PATH" ]]; then
  printf 'APK not found at %s. Use --build to force a build.\n' "$APK_PATH" >&2
  exit 4
fi

printf 'Installing %s on %s...\n' "$APK_PATH" "$target_device"
install_options=(-r -d)
if [[ "$target_is_emulator" == 0 ]]; then
  install_options=(--no-streaming "${install_options[@]}")
fi
"$ADB_BIN" -s "$target_device" install "${install_options[@]}" "$APK_PATH"
printf 'Launching %s...\n' "$PACKAGE_NAME"
"$ADB_BIN" -s "$target_device" shell monkey -p "$PACKAGE_NAME" -c android.intent.category.LAUNCHER 1 >/dev/null
printf 'Practice app launched on %s.\nAPK: %s\nAPI: %s\n' "$target_device" "$APK_PATH" "$EXPO_PUBLIC_PRACTICE_API_URL"
