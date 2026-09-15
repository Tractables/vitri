#!/bin/sh
# Install a release archive of the command-line tool as a user would, and run it.
#
#     check-cli-archive.sh <archive.tar.gz> [image]
#
# With an image, the check runs inside a new docker container of that image,
# with no network and only the archive and this script mounted. Without one, it
# runs on this machine with an empty environment.
#
# It verifies the checksum, extracts into a new directory, requires that no Rust
# toolchain is on PATH and that each library in lib/ is the copy the binary
# loads, then runs `vitri --help` and the bundled example, and checks the bundle.
set -eu

if [ "$#" -lt 1 ] || [ "$#" -gt 2 ]; then
  sed -n '2,/^set/{/^set/d;s/^# \{0,1\}//;p}' "$0" >&2
  exit 2
fi

scripts=$(cd "$(dirname "$0")" && pwd)
archive_dir=$(cd "$(dirname "$1")" && pwd)
archive=$(basename "$1")

if [ "$#" -eq 2 ]; then
  echo "checking $archive in a new $2 container"
  exec docker run --rm --network none \
    -v "$archive_dir:/archive:ro" -v "$scripts:/scripts:ro" \
    "$2" /bin/sh /scripts/check-cli-archive.sh "/archive/$archive"
fi

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

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
