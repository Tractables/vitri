#!/usr/bin/env bash
# Download one file from the GNU archive, trying each mirror in turn.
#
#     gnu-fetch.sh <path> <dest> [sha256]
#
# <path> is the file's path under gnu/ on the archive, such as
# gmp/gmp-6.3.0.tar.xz. The mirrors are ftp.gnu.org, then mirrors.kernel.org,
# so a build does not fail while one of them is unreachable. With [sha256], a
# download with any other sha256 counts as a failure and the next mirror is
# tried. The file lands at <dest> only once a mirror has served it whole (and,
# with [sha256], matching), so no partial download is ever left there.
set -euo pipefail

[ "$#" -ge 2 ] && [ "$#" -le 3 ] || { sed -n '2,/^set/{/^set/d;s/^# \{0,1\}//;p}' "$0" >&2; exit 2; }
path=$1
dest=$2
sha256=${3:-}

for mirror in https://ftp.gnu.org/gnu https://mirrors.kernel.org/gnu; do
  if curl -fsSL --retry 3 --connect-timeout 20 -o "$dest.part" "$mirror/$path"; then
    if [ -z "$sha256" ] || echo "$sha256  $dest.part" | sha256sum -c --status; then
      mv "$dest.part" "$dest"
      exit 0
    fi
    echo "$mirror/$path does not have the sha256 $sha256" >&2
  fi
  rm -f "$dest.part"
done
echo "no GNU mirror served $path" >&2
exit 1
