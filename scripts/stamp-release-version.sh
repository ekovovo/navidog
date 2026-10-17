#!/usr/bin/env bash
# On a tag build, rewrite the workspace version to the tag so the binary,
# the update check, and package metadata agree. The checkout is not pushed.
# A nightly or a local run leaves Cargo.toml alone.
set -euo pipefail
cd "$(dirname "$0")/.."

tag="${GITHUB_REF_NAME:-}"
if [[ ! "$tag" =~ ^v[0-9]+\.[0-9]+\.[0-9]+ ]]; then
  echo "stamp-release-version: ${tag:-<unset>} is not a release tag; leaving Cargo.toml"
  exit 0
fi
version="${tag#v}"

tmp=$(mktemp)
awk -v version="$version" '
  BEGIN { in_pkg = 0; in_deps = 0; pkg_done = 0; dep_count = 0 }
  /^\[workspace\.package\]/ { in_pkg = 1; in_deps = 0 }
  /^\[workspace\.dependencies\]/ { in_pkg = 0; in_deps = 1 }
  /^\[/ && $0 !~ /^\[workspace\.(package|dependencies)\]/ { in_pkg = 0; in_deps = 0 }
  in_pkg && /^version = "/ {
    sub(/"[^"]+"/, "\"" version "\"")
    pkg_done = 1
  }
  in_deps && /^navidog/ && /version = "/ {
    sub(/version = "[^"]+"/, "version = \"" version "\"")
    dep_count++
  }
  { print }
  END {
    if (!pkg_done) exit 2
    if (dep_count < 1) exit 3
  }
' Cargo.toml > "$tmp"
mv "$tmp" Cargo.toml
echo "stamped Cargo.toml to $version"
