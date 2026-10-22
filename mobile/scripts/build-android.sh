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
LIB=$(ls "target/$PROFILE"/libhush_ffi.{so,dylib} 2>/dev/null | head -1)
cargo run -p hush-ffi --features cli --bin uniffi-bindgen --locked -- \
  generate --library "$LIB" --language kotlin --out-dir "$APP/java" --no-format

# UniFFI declares HushException.App's `message` field next to an overriding `message` getter,
# which Kotlin rejects as a redeclaration: keep the field, overriding Throwable.message.
perl -0pi -e 's/val `message`: kotlin\.String\n(\s*)\) : HushException\(\) \{\n\s*override val message\n\s*get\(\) = "message=\$\{ `message` \}"\n/override val `message`: kotlin.String\n$1) : HushException() {\n/g' \
  "$APP/java/uniffi/hush_ffi/hush_ffi.kt"
echo "android: $(du -sh "$APP/jniLibs" | cut -f1) of native libraries, bindings in $APP/java/uniffi"
