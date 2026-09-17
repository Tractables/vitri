#!/usr/bin/env bash
# Put the files the browser page is served with into one directory.
#
#     site.sh <out-dir> [emscripten-prefix]
#
# <out-dir> receives the page: index.html, styles.css, the scripts, and the two
# example formulas, copied rather than linked. With <emscripten-prefix>, the
# prefix emscripten-prefix.sh installed GMP into, it also receives the built
# module from target/wasm32-unknown-emscripten/release, the two side modules the
# module loads from that prefix, and the notices under notices/.
#
# The docs workflow assembles the published site from this directory, and
# bindings/wasm/README.md serves it from a local build.
set -euo pipefail

if [ "$#" -lt 1 ] || [ "$#" -gt 2 ]; then
  sed -n '2,/^set/{/^set/d;s/^# \{0,1\}//;p}' "$0" >&2
  exit 2
fi

out=$1
here=$(cd "$(dirname "$0")" && pwd)

mkdir -p "$out"
cp -L "$here"/index.html "$here"/styles.css "$here"/*.js \
  "$here"/example.cnf "$here"/mc2023_track1_008.reduced.cnf "$out/"

if [ "$#" -eq 2 ]; then
  prefix=$(cd "$2" && pwd)
  cp "$here"/target/wasm32-unknown-emscripten/release/vitri.{js,wasm} \
    "$prefix"/lib/libgmp.so "$prefix"/lib/libgmpxx.so "$out/"
  "$here/../../.github/scripts/collect-notices.sh" "$out/notices" "$prefix"
fi
