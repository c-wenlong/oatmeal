#!/usr/bin/env bash
# Runs the Swift tests with coverage and exports lcov for Codecov.
#
# SwiftPM writes coverage into an architecture-specific build directory and names
# the test bundle after the package, so both are discovered rather than
# hard-coded — otherwise this breaks the first time it runs on a different arch.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="${1:-$ROOT/coverage/swift-lcov.info}"

cd "$ROOT/sidecar"
swift test --enable-code-coverage

BUNDLE="$(find .build -maxdepth 3 -name '*.xctest' | head -1)"
if [[ -z "$BUNDLE" ]]; then
  echo "error: no .xctest bundle found — did the tests build?" >&2
  exit 1
fi
BINARY="$BUNDLE/Contents/MacOS/$(basename "$BUNDLE" .xctest)"
PROFDATA="$(dirname "$(swift test --show-codecov-path)")/default.profdata"

mkdir -p "$(dirname "$OUT")"
xcrun llvm-cov export -format=lcov "$BINARY" \
  -instr-profile "$PROFDATA" \
  -ignore-filename-regex='(Tests/|\.build/)' \
  > "$OUT"

# llvm-cov emits absolute paths; Codecov matches on repo-relative ones.
python3 - "$OUT" "$ROOT" <<'PY'
import pathlib, sys
out, root = pathlib.Path(sys.argv[1]), sys.argv[2].rstrip("/") + "/"
out.write_text(out.read_text().replace("SF:" + root, "SF:"))
PY

echo "==> Swift coverage: $OUT ($(grep -c '^SF:' "$OUT") files)"
