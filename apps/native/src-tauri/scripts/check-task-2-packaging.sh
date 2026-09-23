#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TAURI_DIR="${SCRIPT_DIR}/.."
CONFIG="${TAURI_DIR}/tauri.conf.json"
FIXTURE="${SCRIPT_DIR}/fixtures/task-2-app-layout.json"
TARGET=""
FIXTURE_ONLY=false

usage() {
  cat <<'EOF'
Usage: check-task-2-packaging.sh --target TARGET
       check-task-2-packaging.sh --fixture

Validate the checked-in Tauri externalBin declaration and, for a target,
require its staged executable and SHA-256 sidecar before release packaging.
EOF
}

fail() {
  echo "check-task-2-packaging.sh: $*" >&2
  exit 1
}

while (($#)); do
  case "$1" in
    --target)
      (($# >= 2)) || fail "--target requires a value"
      TARGET="$2"
      shift
      ;;
    --fixture)
      FIXTURE_ONLY=true
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

python3 - "$CONFIG" <<'PY'
import json
import sys
from pathlib import Path

config_path = Path(sys.argv[1])
config = json.loads(config_path.read_text())
external_bin = config.get("bundle", {}).get("externalBin", [])
if "binaries/whisper-cli" not in external_bin:
    raise SystemExit(
        "check-task-2-packaging.sh: tauri.conf.json must declare "
        'bundle.externalBin "binaries/whisper-cli"'
    )
PY

python3 - "$FIXTURE" <<'PY'
import json
import sys
from pathlib import Path

fixture = json.loads(Path(sys.argv[1]).read_text())
expected = {
    "targets": {
        "aarch64-apple-darwin": {
            "bundle_relative_path": "Contents/MacOS/whisper-cli",
            "architecture": "arm64",
            "executable": True,
        },
        "x86_64-apple-darwin": {
            "bundle_relative_path": "Contents/MacOS/whisper-cli",
            "architecture": "x86_64",
            "executable": True,
        },
    }
}
if fixture != expected or set(fixture["targets"]) != {
    "aarch64-apple-darwin", "x86_64-apple-darwin"
}:
    raise SystemExit(
        "check-task-2-packaging.sh: app-layout fixture must validate both "
        "arm64 and x86_64 target executables"
    )
PY

if [[ "$FIXTURE_ONLY" == true ]]; then
  [[ -z "$TARGET" ]] || fail "--fixture cannot be combined with --target"
  echo "Task 2 config and deterministic app-layout fixture validated"
  exit 0
fi

[[ -n "$TARGET" ]] || fail "--target is required for the staging check"
bash "${SCRIPT_DIR}/build-whisper-cli.sh" \
  --check-only --require-staged --target "$TARGET"
echo "Task 2 config and staged ${TARGET} artifact validated"
