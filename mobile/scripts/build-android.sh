#!/usr/bin/env bash
# Builds the hush engine (crates/hush-ffi) for Android and generates its Kotlin bindings:
#   mobile/android/app/src/main/jniLibs/<abi>/libhush_ffi.so
#   mobile/android/app/src/main/java/uniffi/hush_ffi/hush_ffi.kt
# Needs the Android NDK (ANDROID_NDK_HOME) and `cargo install cargo-ndk`.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
APP="$ROOT/mobile/android/app/src/main"
PROFILE=${PROFILE:-ffi}
ABIS=${ABIS:-arm64-v8a x86_64}
cd "$ROOT"

targets=()
for abi in $ABIS; do targets+=(-t "$abi"); done
cargo ndk "${targets[@]}" --platform 29 -o "$APP/jniLibs" build -p hush-ffi --profile "$PROFILE" --locked

# The bindings come from the host build of the same crate (UniFFI reads its metadata).
cargo build -p hush-ffi --profile "$PROFILE" --locked
# Only one of the two exists, and `ls` fails on the other: pick whichever is there.
LIB=""
for f in "target/$PROFILE/libhush_ffi.so" "target/$PROFILE/libhush_ffi.dylib"; do
  [ -f "$f" ] && LIB=$f && break
done
cargo run -p hush-ffi --features cli --bin uniffi-bindgen --locked -- \
  generate --library "$LIB" --language kotlin --out-dir "$APP/java" --no-format

echo "android: $(du -sh "$APP/jniLibs" | cut -f1) of native libraries, bindings in $APP/java/uniffi"
