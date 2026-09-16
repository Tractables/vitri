#!/bin/sh
# Copy the notices a Python distribution of vitri carries into notices/, where
# the `license-files` of pyproject.toml finds them.
#
#   collect-notices.sh                     the licence and the third-party notices
#   collect-notices.sh --wheel <gmp-prefix>
#                                          those, and the licence texts of the GMP
#                                          that .github/scripts/gmp.sh installed in
#                                          <gmp-prefix>, which a repaired Linux
#                                          wheel bundles
set -eu

usage() {
    echo "usage: $0 [--wheel <gmp-prefix>]" >&2
    exit 2
}

here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
notices="$here/notices"

case "${1:-}" in
"") [ "$#" -eq 0 ] || usage ;;
--wheel) [ "$#" -eq 2 ] || usage ;;
*) usage ;;
esac

mkdir -p "$notices"
cp "$root/LICENSE" "$root/docs/THIRD-PARTY.md" "$notices/"

if [ "${1:-}" = --wheel ]; then
    # libgmp and libgmpxx, both from the GMP sources, under LGPLv3+ or GPLv2+.
    # A GMP- prefix keeps the texts apart from vitri's own files.
    texts="$2/share/licenses/gmp"
    ls "$texts"/* > /dev/null 2>&1 || { echo "$texts holds no GMP licence texts" >&2; exit 1; }
    for text in "$texts"/*; do
        cp "$text" "$notices/GMP-$(basename "$text")"
    done
fi
