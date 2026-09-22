#!/usr/bin/env bash
set -euo pipefail

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "This script must run on macOS." >&2
  exit 1
fi

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RELEASE_DIR="$PROJECT_ROOT/release"
VERSION="$(node -p "require('${PROJECT_ROOT}/package.json').version")"

cd "$PROJECT_ROOT"

corepack enable
corepack install
pnpm install --frozen-lockfile
rustup target add aarch64-apple-darwin x86_64-apple-darwin

pnpm tauri build \
  --target universal-apple-darwin \
  --bundles dmg \
  --config '{"bundle":{"createUpdaterArtifacts":false}}'

BUNDLE_ROOT="$PROJECT_ROOT/src-tauri/target/universal-apple-darwin/release/bundle"
APP_PATH="$(find "$BUNDLE_ROOT/macos" -maxdepth 1 -name '*.app' -type d | head -1)"
DMG_PATH="$(find "$BUNDLE_ROOT/dmg" -maxdepth 1 -name '*.dmg' -type f | head -1)"

if [[ -z "$APP_PATH" || -z "$DMG_PATH" ]]; then
  echo "macOS bundle output not found." >&2
  exit 1
fi

mkdir -p "$RELEASE_DIR"
DMG_NAME="PPBind-${VERSION}-macOS-universal-unsigned.dmg"
ZIP_NAME="PPBind-${VERSION}-macOS-universal-unsigned.zip"
cp "$DMG_PATH" "$RELEASE_DIR/$DMG_NAME"
ditto -c -k --sequesterRsrc --keepParent \
  "$APP_PATH" \
  "$RELEASE_DIR/$ZIP_NAME"

(
  cd "$RELEASE_DIR"
  shasum -a 256 "$DMG_NAME" "$ZIP_NAME" > SHA256SUMS-macos.txt
)

echo "macOS release artifacts: $RELEASE_DIR"
