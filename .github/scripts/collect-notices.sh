#!/usr/bin/env bash
# Copy the notices anything built from this repository ships with into one
# directory.
#
#     collect-notices.sh <out-dir> [gmp-prefix]
#     collect-notices.sh list [gmp]
#
# The `list` form names the files the first form writes, one per line, for a
# check that has the directory and wants to know what belongs in it; `gmp` adds
# the GMP texts. It reads nothing and needs nothing installed.
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

usage() {
  sed -n '2,/^set/{/^set/d;s/^# \{0,1\}//;p}' "$0" >&2
  exit 2
}

# THE notice list: what every build ships, and what a build that ships the GMP
# libraries adds. The copies below are named from it and so is the `list` form,
# so a check cannot look for a file this does not write.
notices=(LICENSE THIRD-PARTY.md goatd-THIRD-PARTY.md)
gmp_notices=(GMP-COPYING.LESSERv3 GMP-COPYINGv3 GMP-COPYINGv2)

if [ "${1:-}" = list ]; then
  [ "$#" -le 2 ] || usage
  printf '%s\n' "${notices[@]}"
  [ "${2:-}" != gmp ] || printf '%s\n' "${gmp_notices[@]}"
  exit 0
fi

if [ "$#" -lt 1 ] || [ "$#" -gt 2 ]; then
  usage
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
  for notice in "${gmp_notices[@]}"; do
    name=${notice#GMP-}
    [ -s "$texts/$name" ] || { echo "$texts/$name is missing; gmp.sh build installs it" >&2; exit 1; }
    cp "$texts/$name" "$out/$notice"
  done
fi

ls "$out"
