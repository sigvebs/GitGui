#!/bin/bash
# Wraps the release binary in a .app so it can live in /Applications and get a
# proper Dock entry instead of behaving like a terminal program.
set -euo pipefail

cd "$(dirname "$0")/.."

NAME="Git GUI"
BUNDLE="dist/$NAME.app"

cargo build --release

rm -rf "$BUNDLE"
mkdir -p "$BUNDLE/Contents/MacOS" "$BUNDLE/Contents/Resources"
cp target/release/gitgui "$BUNDLE/Contents/MacOS/gitgui"

ICON_ENTRY=""
if [ -f scripts/icon.icns ]; then
    cp scripts/icon.icns "$BUNDLE/Contents/Resources/icon.icns"
    ICON_ENTRY="
    <key>CFBundleIconFile</key>
    <string>icon</string>"
fi

cat > "$BUNDLE/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>
    <string>$NAME</string>
    <key>CFBundleDisplayName</key>
    <string>$NAME</string>
    <key>CFBundleIdentifier</key>
    <string>local.gitgui</string>
    <key>CFBundleVersion</key>
    <string>0.1.0</string>
    <key>CFBundleShortVersionString</key>
    <string>0.1.0</string>
    <key>CFBundleExecutable</key>
    <string>gitgui</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>LSMinimumSystemVersion</key>
    <string>11.0</string>
    <key>NSHighResolutionCapable</key>
    <true/>$ICON_ENTRY
</dict>
</plist>
PLIST

echo "Built $BUNDLE"
echo
echo "Launched from Finder it opens the last repository you used, since it has"
echo "no working directory to inspect. Pass a path on the command line, or use"
echo "Repository > Open, to pick a different one."
