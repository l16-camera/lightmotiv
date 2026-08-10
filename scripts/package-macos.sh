#!/usr/bin/env bash
# Build a double-clickable Luminat.app (no tauri-cli required).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

echo "==> libcp-export (x86_64)"
make libcp-export

echo "==> light + lumen release"
cargo build --release -p light -p lumen

APP="$ROOT/dist/Luminat.app"
BIN="$ROOT/target/release/lumen"
HELPER="$ROOT/tools/libcp-export/libcp-export"
VERSION="$(cat VERSION 2>/dev/null || echo 0.0.0)"

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"

cp "$BIN" "$APP/Contents/MacOS/Luminat"
cp "$HELPER" "$APP/Contents/MacOS/libcp-export"
chmod +x "$APP/Contents/MacOS/Luminat" "$APP/Contents/MacOS/libcp-export"

# Optional: stage empty libcp dir for user drop-in
mkdir -p "$APP/Contents/Resources/libcp"
cat > "$APP/Contents/Resources/libcp/README.txt" << 'EOF'
Place libcp.dylib and libceres.dylib here (from Lumen.app/Contents/Frameworks),
or install Lumen.app, or use in-app Setup to pick the Frameworks folder.
EOF

# Icon
if [[ -f lumen/src-tauri/icons/icon.icns ]]; then
  cp lumen/src-tauri/icons/icon.icns "$APP/Contents/Resources/AppIcon.icns"
fi

cat > "$APP/Contents/Info.plist" << EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key>
  <string>en</string>
  <key>CFBundleExecutable</key>
  <string>Luminat</string>
  <key>CFBundleIdentifier</key>
  <string>dev.blmk.luminat</string>
  <key>CFBundleInfoDictionaryVersion</key>
  <string>6.0</string>
  <key>CFBundleName</key>
  <string>Luminat</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleShortVersionString</key>
  <string>${VERSION}</string>
  <key>CFBundleVersion</key>
  <string>${VERSION}</string>
  <key>LSMinimumSystemVersion</key>
  <string>12.0</string>
  <key>NSHighResolutionCapable</key>
  <true/>
  <key>CFBundleIconFile</key>
  <string>AppIcon</string>
</dict>
</plist>
EOF

# ad-hoc sign so Gatekeeper is less angry on local builds
if command -v codesign >/dev/null; then
  codesign --force --deep -s - "$APP" 2>/dev/null || true
fi

echo ""
echo "Built: $APP"
echo "Open:  open \"$APP\""
echo ""
echo "First launch: setup wizard if libcp missing."
echo "Requires Rosetta for libcp-export (x86_64)."
echo "  softwareupdate --install-rosetta"
