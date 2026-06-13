#!/usr/bin/env bash
set -euo pipefail

# 脚本位于 scripts/，解析出 repo 根目录
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$ROOT"

APP_NAME="FFF Viewer"
BIN_NAME="fff_viewer"
BUNDLE_ID="me.clcy.fff-viewer"
VERSION="$(grep -m1 '^version' Cargo.toml | sed -E 's/version *= *"([^"]+)".*/\1/')"

DIST="$ROOT/dist"
APP_DIR="$DIST/$APP_NAME.app"
CONTENTS="$APP_DIR/Contents"
MACOS_DIR="$CONTENTS/MacOS"
RES_DIR="$CONTENTS/Resources"

echo "ROOT=$ROOT"
echo "VERSION=$VERSION"
