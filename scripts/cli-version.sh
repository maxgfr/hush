#!/usr/bin/env bash
# Sets the hush version in crates/hush-cli/Cargo.toml, in the hush-cli entry of Cargo.lock (so
# that `cargo build --locked` still accepts the lockfile) and in web/package.json (the version the
# web app shows).
# Usage: scripts/cli-version.sh <semver>
set -euo pipefail

version="${1:?usage: scripts/cli-version.sh <version>}"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$ ]] ||
  { echo "not a semver version: $version" >&2; exit 1; }

cd "$(dirname "$0")/.."

# Rewrites the first `version = ...` line that follows a line matching
# `after` (the start of the file when empty), and fails if there is none.
set_version() {
  local file=$1 after=$2
  awk -v v="$version" -v after="$after" '
    BEGIN { armed = (after == "") }
    armed && !done && /^version = / { $0 = "version = \"" v "\""; done = 1 }
    after != "" && $0 == after { armed = 1 }
    { print }
    END { exit !done }' "$file" > "$file.tmp" ||
    { rm -f "$file.tmp"; echo "no version line to update in $file" >&2; exit 1; }
  mv "$file.tmp" "$file"
}

# The first `version = ...` line of Cargo.toml is the [package] one.
set_version crates/hush-cli/Cargo.toml ""
set_version Cargo.lock 'name = "hush-cli"'
# The first "version" of package.json is the package's own.
perl -0pi -e 's/"version": "[^"]*"/"version": "'"$version"'"/' web/package.json
grep -q "\"version\": \"$version\"" web/package.json ||
  { echo "no version to update in web/package.json" >&2; exit 1; }
echo "hush version set to $version"
