#!/usr/bin/env bash
# Builds the Swift sidecar and stages it where Tauri expects an externalBin.
#
# Tauri resolves external binaries by appending the target triple, so the staged
# filename must carry the suffix even though the bundler strips it again inside
# the .app.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TRIPLE="aarch64-apple-darwin"
CONFIG="${1:-release}"
DEST="$ROOT/src-tauri/binaries"

echo "==> Building sidecar ($CONFIG)"
cd "$ROOT/sidecar"
swift build -c "$CONFIG"

BUILT="$ROOT/sidecar/.build/$CONFIG/OatmealSidecar"
if [[ ! -f "$BUILT" ]]; then
  echo "error: expected binary at $BUILT" >&2
  exit 1
fi

mkdir -p "$DEST"
cp "$BUILT" "$DEST/oatmeal-sidecar-$TRIPLE"
chmod +x "$DEST/oatmeal-sidecar-$TRIPLE"

echo "==> Staged $DEST/oatmeal-sidecar-$TRIPLE"
