#!/bin/sh
# Check a release archive of the command-line tool.
#
#     check-cli-archive.sh <archive.tar.gz> [image]
#     check-cli-archive.sh --gmp-prefix <dir> <archive.tar.gz>
#
# The first form installs the archive as a user would, and runs it. With an
# image, the check runs inside a new docker container of that image, with no
# network and only the archive and this scripts directory mounted. Without one,
# it runs on this machine with an empty environment. It verifies the checksum,
# extracts into a new directory, requires that no Rust toolchain is on PATH, that
# each library in lib/ is the copy the binary loads and that notices/ holds every
# file collect-notices.sh names, then runs `vitri --help` and the bundled
# example, and checks the bundle.
#
# The second form runs on the build machine, against the prefix gmp.sh installed
# GMP into. It requires that each library in lib/ has the GNU build ID of the
# same library in <dir>/lib, that bin/vitri has the RUNPATH $ORIGIN/../lib, and
# that bin/vitri defines none of the symbols those libraries export, so GMP is
# not linked into it. Needs readelf and nm.
set -eu

usage() {
  sed -n '2,/^set/{/^set/d;s/^# \{0,1\}//;p}' "$0" >&2
  exit 2
}

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

scripts=$(cd "$(dirname "$0")" && pwd)

if [ "${1:-}" = --gmp-prefix ]; then
  [ "$#" -eq 3 ] || usage
  export LC_ALL=C
  prefix=$(cd "$2" && pwd)
  archive_dir=$(cd "$(dirname "$3")" && pwd)
  archive=$(basename "$3")
  work=$(mktemp -d)
  trap 'rm -rf "$work"' EXIT

  tar -xzf "$archive_dir/$archive" --no-same-owner -C "$work" || fail "extracting $archive"
  root="$work/${archive%.tar.gz}"
  binary="$root/bin/vitri"
  ls "$root"/lib/libgmp.so.* > /dev/null 2>&1 || fail "$archive has no GMP in lib/"

  for lib in "$root"/lib/*; do
    (cd "$root" && "$scripts/same-build-id.sh" "lib/$(basename "$lib")" "$prefix/lib/$(basename "$lib")")
  done

  runpath=$(readelf -d "$binary" | sed -n 's/.*(RUNPATH).*Library runpath: \[\(.*\)\]$/\1/p')
  [ "$runpath" = '$ORIGIN/../lib' ] ||
    fail "bin/vitri has RUNPATH '$runpath', not \$ORIGIN/../lib"
  echo "bin/vitri has RUNPATH $runpath"

  originals=""
  for lib in "$root"/lib/*; do
    originals="$originals $prefix/lib/$(basename "$lib")"
  done
  # shellcheck disable=SC2086
  "$(dirname "$0")/no-gmp-inside.sh" "$binary" $originals || fail "bin/vitri carries GMP inside it"
  echo "ok: $archive carries the GMP in $prefix"
  exit 0
fi

if [ "$#" -lt 1 ] || [ "$#" -gt 2 ]; then
  usage
fi

archive_dir=$(cd "$(dirname "$1")" && pwd)
archive=$(basename "$1")

if [ "$#" -eq 2 ]; then
  echo "checking $archive in a new $2 container"
  exec docker run --rm --network none \
    -v "$archive_dir:/archive:ro" -v "$scripts:/scripts:ro" \
    "$2" /bin/sh /scripts/check-cli-archive.sh "/archive/$archive"
fi

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
clean() {
  env -i PATH=/usr/bin:/bin HOME="$work" "$@"
}

echo "system: $(. /etc/os-release && echo "$PRETTY_NAME"), $(ldd --version | head -n 1)"

for tool in cargo rustc rustup; do
  if clean sh -c "command -v $tool" > /dev/null; then
    fail "$tool is on PATH; the check needs a machine without a Rust toolchain"
  fi
done

(cd "$archive_dir" && sha256sum -c "$archive.sha256") || fail "checksum of $archive"

tar -xzf "$archive_dir/$archive" --no-same-owner -C "$work" || fail "extracting $archive"
root="$work/${archive%.tar.gz}"
[ -x "$root/bin/vitri" ] || fail "$archive has no executable bin/vitri"
ls "$root"/lib/libgmp.so.* > /dev/null 2>&1 || fail "$archive has no GMP in lib/"
for notice in $("$scripts/collect-notices.sh" list gmp); do
  [ -s "$root/notices/$notice" ] || fail "$archive has no notices/$notice"
done

deps=$(clean ldd "$root/bin/vitri") || fail "ldd cannot read bin/vitri"
echo "$deps"
case $deps in
  *"not found"*) fail "bin/vitri needs a library this system does not have" ;;
esac
for lib in "$root"/lib/*; do
  soname=$(basename "$lib")
  loaded=$(echo "$deps" | awk -v soname="$soname" '$1 == soname && $2 == "=>" { print $3 }')
  [ -n "$loaded" ] || fail "bin/vitri does not load lib/$soname"
  [ "$(readlink -f "$loaded")" = "$(readlink -f "$lib")" ] ||
    fail "$soname loads from $loaded, not from the archive"
done

system_copies=$(/sbin/ldconfig -p 2> /dev/null | grep -E 'libgmp|libmpfr' || true)
echo "GMP/MPFR in the system library cache: ${system_copies:-none}"

mkdir "$work/run"
cd "$work/run"
clean "$root/bin/vitri" --help > /dev/null || fail "vitri --help"
clean "$root/bin/vitri" "$root/example.cnf" --out-dir bundle --budget-ms 10000 ||
  fail "vitri on example.cnf"
for file in reduced.cnf preprocess.json vtree.vtree; do
  [ -s "bundle/$file" ] || fail "the run wrote no bundle/$file"
done
grep -q '^p cnf ' bundle/reduced.cnf || fail "bundle/reduced.cnf has no DIMACS header"
echo "ok: $archive installs and runs"
