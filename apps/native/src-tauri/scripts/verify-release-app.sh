#!/usr/bin/env bash
set -euo pipefail

APP_PATH="${1:-}"
TARGET="${2:-}"

fail() {
  echo "verify-release-app.sh: $*" >&2
  exit 1
}

[[ -n "$APP_PATH" && -n "$TARGET" ]] || fail "usage: verify-release-app.sh APP_PATH TARGET"
[[ -d "$APP_PATH" ]] || fail "missing app bundle: $APP_PATH"
[[ "$TARGET" == "aarch64-apple-darwin" || "$TARGET" == "x86_64-apple-darwin" ]] || fail "unsupported target: $TARGET"

case "$TARGET" in
  aarch64-apple-darwin) expected_arch=arm64 ;;
  x86_64-apple-darwin) expected_arch=x86_64 ;;
esac

main="$APP_PATH/Contents/MacOS/Marvis"
whisper="$APP_PATH/Contents/MacOS/whisper-cli"
[[ -x "$main" ]] || fail "missing executable app binary: $main"
[[ -x "$whisper" ]] || fail "missing executable bundled whisper-cli: $whisper"

main_description="$(file -b "$main")"
whisper_description="$(file -b "$whisper")"
[[ "$main_description" == *"$expected_arch"* ]] || fail "app architecture mismatch: $main_description"
[[ "$whisper_description" == *"$expected_arch"* ]] || fail "whisper architecture mismatch: $whisper_description"

# Never report a release as signed based only on a signing identity setting;
# codesign performs the strict verification of the produced app contents.
codesign --verify --deep --strict --verbose=2 "$APP_PATH"
echo "Strict signature and ${expected_arch} app/Whisper architecture verification passed"
