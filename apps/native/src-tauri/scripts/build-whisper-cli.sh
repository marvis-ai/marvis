#!/usr/bin/env bash
set -euo pipefail

SOURCE_REPO="https://github.com/ggml-org/whisper.cpp.git"
SOURCE_REV="v1.9.2"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUTPUT_DIR="${SCRIPT_DIR}/../binaries"

EXPECTED_TARGETS=(
  "aarch64-apple-darwin"
  "x86_64-apple-darwin"
  "x86_64-pc-windows-msvc"
  "x86_64-unknown-linux-gnu"
)

usage() {
  cat <<'EOF'
Usage: build-whisper-cli.sh [--check-only] [--require-staged] [--target TARGET]
       build-whisper-cli.sh --stage ARTIFACT --target TARGET [--checksum CHECKSUM]
       build-whisper-cli.sh --dev-stage --target TARGET

Build the pinned whisper.cpp whisper-cli executable for the runner's OS, or
stage a CI artifact using Tauri's target-triple external-binary convention.
TARGET must be aarch64-apple-darwin, x86_64-apple-darwin,
x86_64-pc-windows-msvc, or x86_64-unknown-linux-gnu.
EOF
}

fail() {
  echo "build-whisper-cli.sh: $*" >&2
  exit 1
}

is_expected_target() {
  case "$1" in
    aarch64-apple-darwin|x86_64-apple-darwin|x86_64-pc-windows-msvc|x86_64-unknown-linux-gnu) return 0 ;;
    *) return 1 ;;
  esac
}

os_family_for_target() {
  case "$1" in
    *-apple-darwin) printf '%s\n' "macos" ;;
    *-windows-msvc) printf '%s\n' "windows" ;;
    *-linux-gnu) printf '%s\n' "linux" ;;
    *) fail "unsupported target: ${1}" ;;
  esac
}

host_os_family() {
  case "$(uname -s)" in
    Darwin) printf '%s\n' "macos" ;;
    Linux) printf '%s\n' "linux" ;;
    MINGW*|MSYS*|CYGWIN*) printf '%s\n' "windows" ;;
    *) printf '%s\n' "unknown" ;;
  esac
}

# Substring expected in `file -b` output for the artifact's architecture.
architecture_for_target() {
  case "$1" in
    aarch64-apple-darwin) printf '%s\n' "arm64" ;;
    x86_64-apple-darwin) printf '%s\n' "x86_64" ;;
    x86_64-pc-windows-msvc|x86_64-unknown-linux-gnu) printf '%s\n' "x86-64" ;;
    *) fail "unsupported target: ${1}" ;;
  esac
}

sha256_file() {
  local digest
  if command -v sha256sum >/dev/null 2>&1; then
    digest="$(sha256sum -b "$1")"
  else
    digest="$(shasum -a 256 -b "$1")"
  fi
  # Emit only the digest: -b pins binary-mode reads (a text-mode read would
  # translate CRLF and hash different bytes), and a leading `\` marks an
  # escaped filename — paths with backslashes (e.g. D:\a\_temp\...) would
  # otherwise yield "\hash" and corrupt comparisons.
  printf '%s\n' "$digest" | awk '{ print $1 }' | sed 's/^\\//'
}

parallel_jobs() {
  if command -v nproc >/dev/null 2>&1; then
    nproc
  elif command -v sysctl >/dev/null 2>&1; then
    sysctl -n hw.ncpu
  else
    printf '%s\n' "${NUMBER_OF_PROCESSORS:-4}"
  fi
}

validate_metadata() {
  [[ "$SOURCE_REPO" == "https://github.com/ggml-org/whisper.cpp.git" ]] \
    || fail "unexpected source repository: ${SOURCE_REPO}"
  [[ "$SOURCE_REV" == "v1.9.2" ]] || fail "source revision must be v1.9.2"
  [[ "${EXPECTED_TARGETS[*]}" == "aarch64-apple-darwin x86_64-apple-darwin x86_64-pc-windows-msvc x86_64-unknown-linux-gnu" ]] \
    || fail "unexpected output target names"
  [[ "$(architecture_for_target "${EXPECTED_TARGETS[0]}")" == "arm64" ]] \
    || fail "aarch64-apple-darwin must map to arm64 for file validation"
  [[ "$(architecture_for_target "${EXPECTED_TARGETS[1]}")" == "x86_64" ]] \
    || fail "x86_64-apple-darwin must map to x86_64 for file validation"
  [[ "$(architecture_for_target "${EXPECTED_TARGETS[2]}")" == "x86-64" ]] \
    || fail "x86_64-pc-windows-msvc must map to x86-64 for file validation"
  [[ "$(architecture_for_target "${EXPECTED_TARGETS[3]}")" == "x86-64" ]] \
    || fail "x86_64-unknown-linux-gnu must map to x86-64 for file validation"
}

# Tauri looks for `whisper-cli-<triple>` next to `binaries/whisper-cli` —
# plus `.exe` on Windows.
artifact_for_target() {
  local suffix=""
  [[ "$(os_family_for_target "$1")" == "windows" ]] && suffix=".exe"
  printf '%s/%s\n' "$OUTPUT_DIR" "whisper-cli-$1${suffix}"
}

# Executable-format validation: `file` when the runner ships it (macOS,
# Linux, git-sdk Bash), magic bytes otherwise.
validate_format() {
  local artifact="$1"
  local target="$2"
  local os_family="$3"
  local architecture="$4"

  if command -v file >/dev/null 2>&1; then
    local file_output
    file_output="$(file -b "$artifact")"
    [[ "$file_output" == *"${architecture}"* ]] \
      || fail "artifact architecture mismatch for ${target}: ${file_output}"
    return 0
  fi

  case "$os_family" in
    windows)
      [[ "$(dd if="$artifact" bs=2 count=1 2>/dev/null)" == "MZ" ]] \
        || fail "artifact is not a PE executable: ${artifact}"
      ;;
    linux)
      [[ "$(dd if="$artifact" bs=4 count=1 2>/dev/null | od -An -tx1 | tr -d ' ')" == "7f454c46" ]] \
        || fail "artifact is not an ELF executable: ${artifact}"
      ;;
    *) ;;
  esac
}

# Self-containment: BUILD_SHARED_LIBS=OFF must leave no libwhisper/libggml
# dynamic deps. macOS reads `otool -L`, Linux `ldd`; the Windows MSVC
# build has no equivalent enumeration here (whisper/ggml are .lib static
# deps — the CRT/Win32 DLLs it does reference are system-provided).
validate_linkage() {
  local artifact="$1"
  local os_family="$2"
  local linked_libraries
  case "$os_family" in
    macos)
      command -v otool >/dev/null 2>&1 || fail "otool is required to validate ${artifact}"
      linked_libraries="$(otool -L "$artifact")"
      ;;
    linux)
      command -v ldd >/dev/null 2>&1 || fail "ldd is required to validate ${artifact}"
      linked_libraries="$(ldd "$artifact" 2>/dev/null || true)"
      ;;
    windows)
      return 0
      ;;
    *) ;;
  esac
  [[ "$linked_libraries" != *libwhisper* && "$linked_libraries" != *libggml* ]] \
    || fail "artifact is not self-contained; static whisper/ggml linkage required"
}

validate_binary() {
  local artifact="$1"
  local target="$2"
  local checksum="$3"
  local os_family architecture expected_checksum actual_checksum
  os_family="$(os_family_for_target "$target")"
  architecture="$(architecture_for_target "$target")"

  [[ -f "$artifact" ]] || fail "missing artifact: ${artifact}"
  if [[ "$os_family" != "windows" ]]; then
    [[ -x "$artifact" ]] || fail "artifact is not executable: ${artifact}"
  fi
  validate_format "$artifact" "$target" "$os_family" "$architecture"
  validate_linkage "$artifact" "$os_family"
  [[ -f "$checksum" ]] || fail "missing checksum: ${checksum}"
  expected_checksum="$(awk 'NF { print $1; exit }' "$checksum" | tr -d '[:space:]' | sed 's/^\\//')"
  actual_checksum="$(sha256_file "$artifact")"
  [[ -n "$expected_checksum" && "$actual_checksum" == "$expected_checksum" ]] \
    || { echo "expected: ${expected_checksum:-<empty>} | actual: ${actual_checksum:-<empty>}" >&2; fail "checksum verification failed: ${checksum}"; }
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

  # download-artifact drops Unix permission bits; the staged copy is chmod'd
  # regardless, so restore the bit on the input before validation.
  chmod 755 "$source"
  validate_binary "$source" "$target" "$source_checksum"
  mkdir -p "$OUTPUT_DIR"
  staged="$(artifact_for_target "$target")"
  cp "$source" "$staged"
  chmod 755 "$staged"
  sha256_file "$staged" > "${staged}.sha256"
  validate_artifact "$target"
  echo "Staged ${staged}"
}

stage_dev_artifact() {
  local target="$1"
  local destination_dir
  local destination
  local artifact

  destination_dir="${SCRIPT_DIR}/../target/debug"
  destination="${destination_dir}/$(basename "$(artifact_for_target "$target")")"
  artifact="$(artifact_for_target "$target")"

  if [[ ! -e "$artifact" && ! -e "${artifact}.sha256" ]]; then
    echo "No pinned ${target} Whisper CLI artifact found; using PATH/Homebrew/user fallback."
    exit 0
  fi

  validate_artifact "$target"

  mkdir -p "$destination_dir"
  DEV_STAGE_TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/marvis-whisper-dev.XXXXXX")"
  trap 'rm -rf "${DEV_STAGE_TMP_DIR:-}"' EXIT
  cp "$artifact" "${DEV_STAGE_TMP_DIR}/$(basename "$destination")"
  chmod 755 "${DEV_STAGE_TMP_DIR}/$(basename "$destination")"
  mv "${DEV_STAGE_TMP_DIR}/$(basename "$destination")" "$destination"
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
  case "$(uname -s)-$(uname -m)" in
    Darwin-arm64) target="aarch64-apple-darwin" ;;
    Darwin-x86_64) target="x86_64-apple-darwin" ;;
    Linux-x86_64) target="x86_64-unknown-linux-gnu" ;;
    MINGW*-x86_64|MSYS*-x86_64|CYGWIN*-x86_64) target="x86_64-pc-windows-msvc" ;;
    *) fail "cannot infer a supported target from $(uname -s) $(uname -m); use --target" ;;
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

# Source build — runs on the target's own runner (no cross-compile).
host_os="$(host_os_family)"
[[ "$host_os" == "$(os_family_for_target "$target")" ]] \
  || fail "a $(os_family_for_target "$target") runner is required for ${target} (got $(uname -s))"
command -v git >/dev/null 2>&1 || fail "git is required"
command -v cmake >/dev/null 2>&1 || fail "cmake is required"
if ! command -v sha256sum >/dev/null 2>&1 && ! command -v shasum >/dev/null 2>&1; then
  fail "sha256sum or shasum is required"
fi

# A distributable CPU baseline: GGML_NATIVE=OFF pins an explicit ISA set
# rather than the CI machine's, so the binary can't crash on older user
# hardware. AVX2+F16C+FMA covers x86_64 ~2013+; Metal stays macOS-only.
cmake_flags=(
  -DCMAKE_BUILD_TYPE=Release
  -DBUILD_SHARED_LIBS=OFF
  -DWHISPER_BUILD_EXAMPLES=ON
  -DWHISPER_BUILD_TESTS=OFF
)
case "$host_os" in
  macos)
    case "$target" in
      aarch64-apple-darwin) cmake_arch="arm64" ;;
      x86_64-apple-darwin) cmake_arch="x86_64" ;;
      *) fail "unsupported target: ${target}" ;;
    esac
    cmake_flags+=(-DCMAKE_OSX_ARCHITECTURES="$cmake_arch" -DGGML_METAL=ON)
    ;;
  linux|windows)
    cmake_flags+=(
      -DGGML_NATIVE=OFF -DGGML_METAL=OFF -DGGML_BLAS=OFF
      -DGGML_AVX2=ON -DGGML_F16C=ON -DGGML_FMA=ON
    )
    if [[ "$host_os" == "windows" ]]; then
      cmake_flags+=(-A x64)
    fi
    ;;
esac

mkdir -p "$OUTPUT_DIR"
tmp_dir="$(mktemp -d "${TMPDIR:-/tmp}/whisper-cli.XXXXXX")"
trap 'rm -rf "$tmp_dir"' EXIT
source_dir="$tmp_dir/whisper.cpp"
build_dir="$tmp_dir/build"

git clone --quiet --depth 1 --branch "$SOURCE_REV" "$SOURCE_REPO" "$source_dir"
actual_rev="$(git -C "$source_dir" describe --tags --exact-match 2>/dev/null || true)"
[[ "$actual_rev" == "$SOURCE_REV" ]] || fail "source checkout is not exactly ${SOURCE_REV} (got ${actual_rev:-unknown})"

cmake -S "$source_dir" -B "$build_dir" "${cmake_flags[@]}"
cmake --build "$build_dir" --config Release --target whisper-cli --parallel "$(parallel_jobs)"

if [[ "$host_os" == "windows" ]]; then
  built_cli="$(find "$build_dir" -type f -name "whisper-cli.exe" -perm -111 -print -quit 2>/dev/null || true)"
  [[ -n "$built_cli" ]] || built_cli="$(find "$build_dir" -type f -name "whisper-cli.exe" -print -quit)"
else
  built_cli="$(find "$build_dir" -type f -name whisper-cli -perm -111 -print -quit)"
fi
[[ -n "$built_cli" ]] || fail "whisper-cli executable was not produced"
artifact="$(artifact_for_target "$target")"
cp "$built_cli" "$artifact"
chmod 755 "$artifact"
sha256_file "$artifact" > "${artifact}.sha256"
validate_artifact "$target"
echo "Built ${artifact}"
