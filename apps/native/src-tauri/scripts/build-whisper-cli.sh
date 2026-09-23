#!/usr/bin/env bash
set -euo pipefail

SOURCE_REPO="https://github.com/ggml-org/whisper.cpp.git"
SOURCE_REV="v1.9.2"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUTPUT_DIR="${SCRIPT_DIR}/../binaries"

EXPECTED_TARGETS=(
  "aarch64-apple-darwin"
  "x86_64-apple-darwin"
)

usage() {
  cat <<'EOF'
Usage: build-whisper-cli.sh [--check-only] [--target TARGET]

Build the pinned whisper.cpp whisper-cli executable for a macOS target.
TARGET must be aarch64-apple-darwin or x86_64-apple-darwin.
EOF
}

fail() {
  echo "build-whisper-cli.sh: $*" >&2
  exit 1
}

is_expected_target() {
  case "$1" in
    aarch64-apple-darwin|x86_64-apple-darwin) return 0 ;;
    *) return 1 ;;
  esac
}

validate_metadata() {
  [[ "$SOURCE_REPO" == "https://github.com/ggml-org/whisper.cpp.git" ]] \
    || fail "unexpected source repository: ${SOURCE_REPO}"
  [[ "$SOURCE_REV" == "v1.9.2" ]] || fail "source revision must be v1.9.2"
  [[ "${EXPECTED_TARGETS[*]}" == "aarch64-apple-darwin x86_64-apple-darwin" ]] \
    || fail "unexpected output target names"
}

artifact_for_target() {
  printf '%s/%s\n' "$OUTPUT_DIR" "whisper-cli-$1"
}

validate_artifact() {
  local target="$1"
  local artifact checksum architecture file_output
  artifact="$(artifact_for_target "$target")"
  checksum="${artifact}.sha256"
  architecture="${target%-apple-darwin}"

  [[ -f "$artifact" ]] || fail "missing artifact: ${artifact}"
  [[ -x "$artifact" ]] || fail "artifact is not executable: ${artifact}"
  command -v file >/dev/null 2>&1 || fail "file is required to validate ${artifact}"
  file_output="$(file -b "$artifact")"
  [[ "$file_output" == *"${architecture}"* ]] \
    || fail "artifact architecture mismatch for ${target}: ${file_output}"
  [[ -f "$checksum" ]] || fail "missing checksum: ${checksum}"
  shasum -a 256 -c "$checksum" >/dev/null \
    || fail "checksum verification failed: ${checksum}"
}

validate_metadata

CHECK_ONLY=false
target=""
while (($#)); do
  case "$1" in
    --check-only)
      CHECK_ONLY=true
      ;;
    --target)
      (($# >= 2)) || fail "--target requires a value"
      target="$2"
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      fail "unknown argument: $1"
      ;;
  esac
  shift
done

if [[ -n "$target" ]] && ! is_expected_target "$target"; then
  fail "unsupported target: ${target}"
fi

if "$CHECK_ONLY"; then
  if [[ -n "$target" ]]; then
    artifact="$(artifact_for_target "$target")"
    if [[ -e "$artifact" || -e "${artifact}.sha256" ]]; then
      validate_artifact "$target"
    fi
  else
    for target in "${EXPECTED_TARGETS[@]}"; do
      artifact="$(artifact_for_target "$target")"
      if [[ -e "$artifact" || -e "${artifact}.sha256" ]]; then
        validate_artifact "$target"
      fi
    done
  fi
  echo "whisper.cpp ${SOURCE_REV} metadata and output names validated"
  exit 0
fi

[[ "$(uname -s)" == "Darwin" ]] || fail "a macOS runner is required for a source build"
command -v git >/dev/null 2>&1 || fail "git is required"
command -v cmake >/dev/null 2>&1 || fail "cmake is required"
command -v shasum >/dev/null 2>&1 || fail "shasum is required"

if [[ -z "$target" ]]; then
  case "$(uname -m)" in
    arm64) target="aarch64-apple-darwin" ;;
    x86_64) target="x86_64-apple-darwin" ;;
    *) fail "cannot infer a supported target from $(uname -m); use --target" ;;
  esac
fi

case "$target" in
  aarch64-apple-darwin) cmake_arch="arm64" ;;
  x86_64-apple-darwin) cmake_arch="x86_64" ;;
  *) fail "unsupported target: ${target}" ;;
esac

mkdir -p "$OUTPUT_DIR"
tmp_dir="$(mktemp -d "${TMPDIR:-/tmp}/whisper-cli.XXXXXX")"
trap 'rm -rf "$tmp_dir"' EXIT
source_dir="$tmp_dir/whisper.cpp"
build_dir="$tmp_dir/build"

git clone --quiet --depth 1 --branch "$SOURCE_REV" "$SOURCE_REPO" "$source_dir"
actual_rev="$(git -C "$source_dir" describe --tags --exact-match 2>/dev/null || true)"
[[ "$actual_rev" == "$SOURCE_REV" ]] || fail "source checkout is not exactly ${SOURCE_REV} (got ${actual_rev:-unknown})"

cmake -S "$source_dir" -B "$build_dir" \
  -DCMAKE_BUILD_TYPE=Release \
  -DCMAKE_OSX_ARCHITECTURES="$cmake_arch" \
  -DWHISPER_BUILD_EXAMPLES=ON \
  -DWHISPER_BUILD_TESTS=OFF \
  -DGGML_METAL=ON
cmake --build "$build_dir" --config Release --target whisper-cli --parallel "$(sysctl -n hw.ncpu)"

built_cli="$(find "$build_dir" -type f -name whisper-cli -perm -111 -print -quit)"
[[ -n "$built_cli" ]] || fail "whisper-cli executable was not produced"
artifact="$(artifact_for_target "$target")"
cp "$built_cli" "$artifact"
chmod 755 "$artifact"
shasum -a 256 "$artifact" > "${artifact}.sha256"
validate_artifact "$target"
echo "Built ${artifact}"
