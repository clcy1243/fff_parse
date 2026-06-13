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

echo "Building $APP_NAME v$VERSION ..."

# 1. 编译 release 二进制
cargo build --release --bin "$BIN_NAME"

# 2. 组装 .app bundle 目录结构
rm -rf "$APP_DIR"
mkdir -p "$MACOS_DIR" "$RES_DIR"
cp "target/release/$BIN_NAME" "$MACOS_DIR/$BIN_NAME"
cp "icons/AppIcon.icns" "$RES_DIR/AppIcon.icns"

# 3. 生成 Info.plist（版本号来自 Cargo.toml 单一真源）
cat > "$CONTENTS/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>$APP_NAME</string>
    <key>CFBundleDisplayName</key><string>$APP_NAME</string>
    <key>CFBundleIdentifier</key><string>$BUNDLE_ID</string>
    <key>CFBundleVersion</key><string>$VERSION</string>
    <key>CFBundleShortVersionString</key><string>$VERSION</string>
    <key>CFBundleExecutable</key><string>$BIN_NAME</string>
    <key>CFBundleIconFile</key><string>AppIcon</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>LSMinimumSystemVersion</key><string>10.15</string>
    <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST

# 4. 校验 plist 合法
plutil -lint "$CONTENTS/Info.plist"

echo "App bundle: $APP_DIR"
