#!/bin/sh
# Builds Lantaran.app into apps/macos/build/.
#
#   apps/macos/scripts/build-app.sh              # this Mac's architecture
#   apps/macos/scripts/build-app.sh --universal  # Apple silicon and Intel
#
# The app is signed ad hoc, which is enough to run it on the Mac that built
# it. Distributing it needs a Developer ID signature and notarization.
set -eu

APP_DIR=$(cd "$(dirname "$0")/.." && pwd)
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' "$APP_DIR/../../core/Cargo.toml" | head -1)

"$APP_DIR/scripts/build-core.sh" "$@"

ARCHS="--arch $(uname -m)"
for arg in "$@"; do
  [ "$arg" = "--universal" ] && ARCHS="--arch arm64 --arch x86_64"
done

echo "Building the app"
# shellcheck disable=SC2086
swift build --package-path "$APP_DIR" -c release $ARCHS
BIN=$(swift build --package-path "$APP_DIR" -c release $ARCHS --show-bin-path)

BUNDLE="$APP_DIR/build/Lantaran.app"
rm -rf "$BUNDLE"
mkdir -p "$BUNDLE/Contents/MacOS" "$BUNDLE/Contents/Resources"
cp "$BIN/Lantaran" "$BUNDLE/Contents/MacOS/Lantaran"

cat > "$BUNDLE/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Lantaran</string>
  <key>CFBundleDisplayName</key><string>Lantaran</string>
  <key>CFBundleIdentifier</key><string>com.github.tulungagung-developer.lantaran</string>
  <key>CFBundleExecutable</key><string>Lantaran</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>LSMinimumSystemVersion</key><string>14.0</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.developer-tools</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSPrincipalClass</key><string>NSApplication</string>
</dict>
</plist>
PLIST

codesign --force --sign - "$BUNDLE"
echo "Built $BUNDLE"
