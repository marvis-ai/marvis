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
Usage: build-whisper-cli.sh [--check-only] [--require-staged] [--target TARGET]
       build-whisper-cli.sh --stage ARTIFACT --target TARGET [--checksum CHECKSUM]
       build-whisper-cli.sh --dev-stage --target TARGET

Build the pinned whisper.cpp whisper-cli executable for a macOS target, or
stage a CI artifact using Tauri's target-triple external-binary convention.
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

architecture_for_target() {
  case "$1" in
    aarch64-apple-darwin) printf '%s\n' "arm64" ;;
    x86_64-apple-darwin) printf '%s\n' "x86_64" ;;
    *) fail "unsupported target: ${1}" ;;
  esac
}

validate_metadata() {
  [[ "$SOURCE_REPO" == "https://github.com/ggml-org/whisper.cpp.git" ]] \
    || fail "unexpected source repository: ${SOURCE_REPO}"
  [[ "$SOURCE_REV" == "v1.9.2" ]] || fail "source revision must be v1.9.2"
  [[ "${EXPECTED_TARGETS[*]}" == "aarch64-apple-darwin x86_64-apple-darwin" ]] \
    || fail "unexpected output target names"
  [[ "$(architecture_for_target "${EXPECTED_TARGETS[0]}")" == "arm64" ]] \
    || fail "aarch64-apple-darwin must map to arm64 for file validation"
  [[ "$(architecture_for_target "${EXPECTED_TARGETS[1]}")" == "x86_64" ]] \
    || fail "x86_64-apple-darwin must map to x86_64 for file validation"
}

artifact_for_target() {
  printf '%s/%s\n' "$OUTPUT_DIR" "whisper-cli-$1"
}

validate_binary() {
  local artifact="$1"
  local target="$2"
  local checksum="$3"
  local architecture file_output linked_libraries expected_checksum actual_checksum
  architecture="$(architecture_for_target "$target")"

  [[ -f "$artifact" ]] || fail "missing artifact: ${artifact}"
  [[ -x "$artifact" ]] || fail "artifact is not executable: ${artifact}"
  command -v file >/dev/null 2>&1 || fail "file is required to validate ${artifact}"
  file_output="$(file -b "$artifact")"
  [[ "$file_output" == *"${architecture}"* ]] \
    || fail "artifact architecture mismatch for ${target}: ${file_output}"
  command -v otool >/dev/null 2>&1 || fail "otool is required to validate ${artifact}"
  linked_libraries="$(otool -L "$artifact")"
  [[ "$linked_libraries" != *libwhisper* && "$linked_libraries" != *libggml* ]] \
    || fail "artifact is not self-contained; static whisper/ggml linkage required"
  [[ -f "$checksum" ]] || fail "missing checksum: ${checksum}"
  expected_checksum="$(awk 'NF { print $1; exit }' "$checksum")"
  actual_checksum="$(shasum -a 256 "$artifact" | awk '{ print $1 }')"
  [[ -n "$expected_checksum" && "$actual_checksum" == "$expected_checksum" ]] \
    || fail "checksum verification failed: ${checksum}"
}

validate_artifact() {
  local target="$1"
  local artifact
  artifact="$(artifact_for_target "$target")"
  validate_binary "$artifact" "$target" "${artifact}.sha256"
}

stage_artifact() {
  local source="$1"
  local target="$2"
  local source_checksum="${3:-${source}.sha256}"
  local staged

  validate_binary "$source" "$target" "$source_checksum"
  mkdir -p "$OUTPUT_DIR"
  staged="$(artifact_for_target "$target")"
  cp "$source" "$staged"
  chmod 755 "$staged"
  shasum -a 256 "$staged" > "${staged}.sha256"
  validate_artifact "$target"
  echo "Staged ${staged}"
}

stage_dev_artifact() {
  local target="$1"
  local destination_dir
  local destination
  local artifact

  destination_dir="${SCRIPT_DIR}/../target/debug"
  destination="${destination_dir}/whisper-cli-${target}"
  artifact="$(artifact_for_target "$target")"

  if [[ ! -e "$artifact" && ! -e "${artifact}.sha256" ]]; then
    echo "No pinned ${target} Whisper CLI artifact found; using PATH/Homebrew/user fallback."
    exit 0
  fi

  validate_artifact "$target"

  mkdir -p "$destination_dir"
  DEV_STAGE_TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/marvis-whisper-dev.XXXXXX")"
  trap 'rm -rf "${DEV_STAGE_TMP_DIR:-}"' EXIT
  cp "$artifact" "${DEV_STAGE_TMP_DIR}/whisper-cli-${target}"
  chmod 755 "${DEV_STAGE_TMP_DIR}/whisper-cli-${target}"
  mv "${DEV_STAGE_TMP_DIR}/whisper-cli-${target}" "$destination"
  echo "Staged development Whisper CLI at ${destination}"
}

validate_metadata

CHECK_ONLY=false
REQUIRE_STAGED=false
DEV_STAGE=false
stage_path=""
stage_checksum=""
target=""
while (($#)); do
  case "$1" in
    --check-only)
      CHECK_ONLY=true
      ;;
    --require-staged)
      REQUIRE_STAGED=true
      ;;
    --dev-stage)
      DEV_STAGE=true
      ;;
    --stage)
      (($# >= 2)) || fail "--stage requires a value"
      stage_path="$2"
      shift
      ;;
    --checksum)
      (($# >= 2)) || fail "--checksum requires a value"
      stage_checksum="$2"
      shift
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

if [[ -z "$target" ]]; then
  case "$(uname -m)" in
    arm64) target="aarch64-apple-darwin" ;;
    x86_64) target="x86_64-apple-darwin" ;;
    *) fail "cannot infer a supported target from $(uname -m); use --target" ;;
  esac
fi

if [[ -n "$target" ]] && ! is_expected_target "$target"; then
  fail "unsupported target: ${target}"
fi

if [[ "$REQUIRE_STAGED" == true && "$CHECK_ONLY" == false ]]; then
  fail "--require-staged requires --check-only"
fi

if [[ -n "$stage_path" ]]; then
  [[ "$CHECK_ONLY" == false ]] || fail "--stage cannot be combined with --check-only"
  [[ -n "$stage_checksum" ]] || stage_checksum="${stage_path}.sha256"
  stage_artifact "$stage_path" "$target" "$stage_checksum"
  exit 0
fi

[[ -z "$stage_checksum" ]] || fail "--checksum requires --stage"

if [[ "$DEV_STAGE" == true ]]; then
  [[ "$CHECK_ONLY" == false ]] || fail "--dev-stage cannot be combined with --check-only"
  stage_dev_artifact "$target"
  exit 0
fi

if "$CHECK_ONLY"; then
  if [[ -n "$target" ]]; then
    artifact="$(artifact_for_target "$target")"
    if [[ "$REQUIRE_STAGED" == true || -e "$artifact" || -e "${artifact}.sha256" ]]; then
      validate_artifact "$target"
    fi
  else
    for target in "${EXPECTED_TARGETS[@]}"; do
      artifact="$(artifact_for_target "$target")"
      if [[ "$REQUIRE_STAGED" == true || -e "$artifact" || -e "${artifact}.sha256" ]]; then
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
  -DBUILD_SHARED_LIBS=OFF \
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
