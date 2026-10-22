#!/usr/bin/env bash
# Builds the hush engine (crates/hush-ffi) for iOS and generates its Swift bindings:
#   mobile/ios/build/HushFFI.xcframework   devices, simulators and this Mac (needs Xcode)
#   mobile/ios/build/macos/                 this Mac only: header, module map, static library
#   mobile/ios/HushKit/Sources/HushFFI/hush_ffi.swift
#
# Without Xcode (Command Line Tools only), only the Mac library is built; test HushKit against it
# with `HUSH_FFI_DIR=$PWD/mobile/ios/build/macos swift test --package-path mobile/ios/HushKit`.
# Needs the Rust targets aarch64-apple-ios and aarch64-apple-ios-sim when Xcode is there.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
IOS="$ROOT/mobile/ios"
PROFILE=${PROFILE:-ffi}
MAC=aarch64-apple-darwin
cd "$ROOT"

if xcodebuild -version >/dev/null 2>&1; then
  XCODE=1
  TARGETS=${TARGETS:-aarch64-apple-ios aarch64-apple-ios-sim $MAC}
else
  XCODE=0
  TARGETS=$MAC
fi
for t in $TARGETS; do
  IPHONEOS_DEPLOYMENT_TARGET=17.0 MACOSX_DEPLOYMENT_TARGET=14.0 \
    cargo build -p hush-ffi --profile "$PROFILE" --target "$t" --locked
done

# UniFFI reads the bindings' metadata from the Mac build of the same crate.
GEN="$ROOT/target/uniffi-swift"
rm -rf "${GEN:?}" && mkdir -p "$GEN"
cargo run -p hush-ffi --features cli --bin uniffi-bindgen --locked -- \
  generate --library "target/$MAC/$PROFILE/libhush_ffi.dylib" --language swift --out-dir "$GEN"

HEADERS="$GEN/headers"
mkdir -p "$HEADERS"
cp "$GEN/hush_ffiFFI.h" "$HEADERS/"
cp "$GEN/hush_ffiFFI.modulemap" "$HEADERS/module.modulemap"

LOCAL="$IOS/build/macos"
rm -rf "${LOCAL:?}" && mkdir -p "$LOCAL"
cp "$HEADERS"/* "target/$MAC/$PROFILE/libhush_ffi.a" "$LOCAL/"

if [ "$XCODE" = 1 ]; then
  OUT="$IOS/build/HushFFI.xcframework"
  rm -rf "${OUT:?}"
  args=()
  for t in $TARGETS; do args+=(-library "target/$t/$PROFILE/libhush_ffi.a" -headers "$HEADERS"); done
  xcodebuild -create-xcframework "${args[@]}" -output "$OUT"
  echo "ios: $OUT"
fi

mkdir -p "$IOS/HushKit/Sources/HushFFI"
cp "$GEN/hush_ffi.swift" "$IOS/HushKit/Sources/HushFFI/hush_ffi.swift"
echo "ios: $LOCAL and HushKit/Sources/HushFFI/hush_ffi.swift"
