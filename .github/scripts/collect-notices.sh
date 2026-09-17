#!/usr/bin/env bash
# Copy the notices anything built from this repository ships with into one
# directory.
#
#     collect-notices.sh <out-dir> [gmp-prefix]
#
# <out-dir> receives vitri's LICENSE, docs/THIRD-PARTY.md for the components
# compiled into vitri, and goatd-THIRD-PARTY.md for those compiled through its
# goatd dependency. With <gmp-prefix>, where gmp.sh installed GMP, it also
# receives GMP's licence texts as GMP-COPYING.LESSERv3, GMP-COPYINGv3 and
# GMP-COPYINGv2, for a build that ships the GMP libraries. The release archive,
# the Python distributions and the browser page each carry this directory.
#
# Needs cargo and python3, which locate goatd.
set -euo pipefail

if [ "$#" -lt 1 ] || [ "$#" -gt 2 ]; then
  sed -n '2,/^set/{/^set/d;s/^# \{0,1\}//;p}' "$0" >&2
  exit 2
fi
out=$1

root=$(cd "$(dirname "$0")/../.." && pwd)
goatd=$(cargo metadata --locked --format-version 1 --manifest-path "$root/Cargo.toml" |
  python3 -c '
import json, os, sys
dirs = [os.path.dirname(p["manifest_path"]) for p in json.load(sys.stdin)["packages"] if p["name"] == "goatd"]
if len(dirs) != 1:
    sys.exit(f"expected one goatd package in the build, found {len(dirs)}")
print(dirs[0])
')

mkdir -p "$out"
cp "$root/LICENSE" "$root/docs/THIRD-PARTY.md" "$out/"
cp "$goatd/docs/THIRD-PARTY.md" "$out/goatd-THIRD-PARTY.md"

if [ "$#" -eq 2 ]; then
  texts=$2/share/licenses/gmp
  for name in COPYING.LESSERv3 COPYINGv3 COPYINGv2; do
    [ -s "$texts/$name" ] || { echo "$texts/$name is missing; gmp.sh build installs it" >&2; exit 1; }
    cp "$texts/$name" "$out/GMP-$name"
  done
fi

ls "$out"
