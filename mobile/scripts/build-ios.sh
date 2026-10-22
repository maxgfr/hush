#!/usr/bin/env bash
# Builds the hush engine (crates/hush-ffi) for iOS and generates its Swift bindings:
#   mobile/ios/build/HushFFI.xcframework   devices, simulators and this Mac (for `swift test`)
#   mobile/ios/HushKit/Sources/HushFFI/hush_ffi.swift
# Needs Xcode (macOS) and the Rust targets aarch64-apple-ios and aarch64-apple-ios-sim.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
IOS="$ROOT/mobile/ios"
PROFILE=${PROFILE:-ffi}
cd "$ROOT"

# The Mac slice lets HushKit build and test with `swift test` on the Mac itself.
TARGETS=${TARGETS:-aarch64-apple-ios aarch64-apple-ios-sim aarch64-apple-darwin}
for t in $TARGETS; do
  IPHONEOS_DEPLOYMENT_TARGET=17.0 cargo build -p hush-ffi --profile "$PROFILE" --target "$t" --locked
done

cargo build -p hush-ffi --profile "$PROFILE" --locked
GEN="$ROOT/target/uniffi-swift"
rm -rf "${GEN:?}" && mkdir -p "$GEN"
cargo run -p hush-ffi --features cli --bin uniffi-bindgen --locked -- \
  generate --library "target/$PROFILE/libhush_ffi.dylib" --language swift --out-dir "$GEN"

HEADERS="$GEN/headers"
mkdir -p "$HEADERS"
cp "$GEN/hush_ffiFFI.h" "$HEADERS/"
cp "$GEN/hush_ffiFFI.modulemap" "$HEADERS/module.modulemap"

OUT="$IOS/build/HushFFI.xcframework"
rm -rf "${OUT:?}"
args=()
for t in $TARGETS; do args+=(-library "target/$t/$PROFILE/libhush_ffi.a" -headers "$HEADERS"); done
xcodebuild -create-xcframework "${args[@]}" -output "$OUT"

mkdir -p "$IOS/HushKit/Sources/HushFFI"
cp "$GEN/hush_ffi.swift" "$IOS/HushKit/Sources/HushFFI/hush_ffi.swift"
echo "ios: $OUT and HushKit/Sources/HushFFI/hush_ffi.swift"
