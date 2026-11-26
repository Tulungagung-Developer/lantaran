#!/bin/sh
# Builds the Rust engine as a static library, generates its Swift bindings,
# and packages both for the Swift package in apps/macos.
#
#   apps/macos/scripts/build-core.sh            # this Mac's architecture, release
#   apps/macos/scripts/build-core.sh --universal  # Apple silicon and Intel
#   apps/macos/scripts/build-core.sh --debug      # faster build, slower engine
set -eu

ROOT=$(cd "$(dirname "$0")/../../.." && pwd)
APP="$ROOT/apps/macos"
CORE="$ROOT/core"
PROFILE=release
UNIVERSAL=0
for arg in "$@"; do
  case "$arg" in
    --universal) UNIVERSAL=1 ;;
    --debug) PROFILE=debug ;;
    *) echo "unknown option: $arg" >&2; exit 2 ;;
  esac
done

if [ "$UNIVERSAL" = 1 ]; then
  TARGETS="aarch64-apple-darwin x86_64-apple-darwin"
else
  case "$(uname -m)" in
    arm64) TARGETS=aarch64-apple-darwin ;;
    *) TARGETS=x86_64-apple-darwin ;;
  esac
fi

PROFILE_FLAG=""
[ "$PROFILE" = release ] && PROFILE_FLAG="--release"

# Matches the deployment target in Package.swift, so the linker does not
# warn that the library was built for a newer macOS.
export MACOSX_DEPLOYMENT_TARGET=14.0

LIBS=""
for target in $TARGETS; do
  echo "Building the engine for $target ($PROFILE)"
  cargo build --manifest-path "$CORE/Cargo.toml" -p lantaran-ffi $PROFILE_FLAG --target "$target"
  LIBS="$LIBS $CORE/target/$target/$PROFILE/liblantaran_ffi.a"
done

WORK="$APP/.build/core"
rm -rf "$WORK"
mkdir -p "$WORK/headers" "$WORK/swift"

# One library for every architecture.
# shellcheck disable=SC2086
lipo -create $LIBS -output "$WORK/liblantaran_ffi.a"

# The interface is read from the built dynamic library.
FIRST=$(echo "$TARGETS" | cut -d' ' -f1)
echo "Generating Swift bindings"
# uniffi-bindgen reads cargo metadata from the current directory.
(cd "$CORE" && cargo run -q -p lantaran-ffi --bin uniffi-bindgen -- \
  generate --library "target/$FIRST/$PROFILE/liblantaran_ffi.dylib" \
  --language swift --out-dir "$WORK/swift")

cp "$WORK/swift/lantaran_ffiFFI.h" "$WORK/headers/"
# xcodebuild looks for a file named module.modulemap.
cp "$WORK/swift/lantaran_ffiFFI.modulemap" "$WORK/headers/module.modulemap"

rm -rf "$APP/Frameworks/LantaranFFI.xcframework"
mkdir -p "$APP/Frameworks"
xcodebuild -create-xcframework \
  -library "$WORK/liblantaran_ffi.a" -headers "$WORK/headers" \
  -output "$APP/Frameworks/LantaranFFI.xcframework" >/dev/null

cp "$WORK/swift/lantaran_ffi.swift" "$APP/Sources/LantaranCore/lantaran_ffi.swift"
echo "Done: apps/macos/Frameworks/LantaranFFI.xcframework"
