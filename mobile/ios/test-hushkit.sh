#!/usr/bin/env bash
# Runs HushKit's tests on this Mac. With Xcode: against the XCFramework, as CI does. With only the
# Command Line Tools: against the Mac library of mobile/scripts/build-ios.sh, with the paths to
# Swift Testing that the Command Line Tools leave out.
set -euo pipefail
IOS=$(cd "$(dirname "$0")" && pwd)
cd "$IOS/HushKit"
if xcodebuild -version >/dev/null 2>&1; then
  exec swift test "$@"
fi
[ -f "$IOS/build/macos/libhush_ffi.a" ] || "$IOS/../scripts/build-ios.sh"
CLT=/Library/Developer/CommandLineTools
export HUSH_FFI_DIR="$IOS/build/macos"
exec swift test --build-system native \
  -Xswiftc -F -Xswiftc "$CLT/Library/Developer/Frameworks" \
  -Xswiftc -plugin-path -Xswiftc "$CLT/usr/lib/swift/host/plugins/testing" \
  -Xlinker -F -Xlinker "$CLT/Library/Developer/Frameworks" \
  -Xlinker -rpath -Xlinker "$CLT/Library/Developer/Frameworks" "$@"
