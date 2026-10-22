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

# Two names in the engine's API clash with Kotlin's own, and Kotlin refuses to compile them:
# - HushException.App's `message` field sits next to an overriding `message` getter: keep the
#   field, overriding Throwable.message;
# - Session.close() returns the farewell frame, while every object is also an AutoCloseable whose
#   close() returns nothing: Kotlin sees it as `closeSession()`.
perl -pi -e 's/fun `close`\(\): kotlin\.ByteArray\?/fun `closeSession`(): kotlin.ByteArray?/g' \
  "$APP/java/uniffi/hush_ffi/hush_ffi.kt"
perl -0pi -e 's/val `message`: kotlin\.String\n(\s*)\) : HushException\(\) \{\n\s*override val message\n\s*get\(\) = "message=\$\{ `message` \}"\n/override val `message`: kotlin.String\n$1) : HushException() {\n/g' \
  "$APP/java/uniffi/hush_ffi/hush_ffi.kt"
echo "android: $(du -sh "$APP/jniLibs" | cut -f1) of native libraries, bindings in $APP/java/uniffi"
