#!/bin/sh
# Copy the notices a Python distribution of vitri carries into notices/, where
# the `license-files` of pyproject.toml finds them.
#
#   collect-notices.sh          the licence and the third-party notices
#   collect-notices.sh --wheel  those, and the copyright file and licence texts
#                               of the GMP build a repaired Linux wheel bundles
#
# The --wheel form reads the Debian and Ubuntu package documentation of the
# library the build linked, so it runs on the system that builds the wheel.
set -eu

here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
notices="$here/notices"

mkdir -p "$notices"
cp "$root/LICENSE" "$root/docs/THIRD-PARTY.md" "$notices/"

case "${1:-}" in
"") ;;
--wheel)
    # libgmp and libgmpxx, both from the GMP sources. GMP is under LGPLv3+ or
    # GPLv2+, and LGPLv3 is written as additional permissions on top of GPLv3.
    cp /usr/share/doc/libgmp10/copyright "$notices/GMP-copyright"
    for licence in LGPL-3 GPL-3 GPL-2; do
        cp "/usr/share/common-licenses/$licence" "$notices/$licence"
    done
    ;;
*)
    echo "usage: $0 [--wheel]" >&2
    exit 2
    ;;
esac
