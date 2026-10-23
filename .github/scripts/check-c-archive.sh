#!/bin/sh
# Check a release archive of the C library.
#
#     check-c-archive.sh --gmp-prefix <dir> <archive.tar.gz>
#     check-c-archive.sh <archive.tar.gz> [image...]
#
# The first form runs on the build machine, against the prefix gmp.sh installed
# GMP into. It requires that each GMP library in lib/ has the GNU build ID of
# the same library in <dir>/lib, that lib/libvitri_c.so has the RUNPATH
# $ORIGIN, and that it defines none of the symbols those libraries export, so
# GMP is not linked into it. Needs readelf and nm.
#
# The second form uses the archive as a consumer would. It verifies the
# checksum, extracts into a new directory, requires the header, the library,
# GMP, the pkg-config file and every notice collect-notices.sh names, and
# builds the C and C++ examples with the flags pkg-config gives. Each program
# must load libvitri_c.so and GMP from the archive and write a bundle for the
# bundled example, here and then in a new docker container of each image, with
# no network and only the extracted archive and the programs mounted. Needs cc,
# c++, pkg-config and ldd.
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

extract() {
  archive_dir=$(cd "$(dirname "$1")" && pwd)
  archive=$(basename "$1")
  work=$(mktemp -d)
  trap 'rm -rf "$work"' EXIT
  tar -xzf "$archive_dir/$archive" --no-same-owner -C "$work" || fail "extracting $archive"
  root="$work/${archive%.tar.gz}"
  library="$root/lib/libvitri_c.so"
  [ -f "$library" ] || fail "$archive has no lib/libvitri_c.so"
  ls "$root"/lib/libgmp.so.* > /dev/null 2>&1 || fail "$archive has no GMP in lib/"
}

if [ "${1:-}" = --gmp-prefix ]; then
  [ "$#" -eq 3 ] || usage
  export LC_ALL=C
  prefix=$(cd "$2" && pwd)
  extract "$3"

  originals=""
  for lib in "$root"/lib/libgmp*; do
    name=$(basename "$lib")
    (cd "$root" && "$scripts/same-build-id.sh" "lib/$name" "$prefix/lib/$name")
    originals="$originals $prefix/lib/$name"
  done

  runpath=$(readelf -d "$library" | sed -n 's/.*(RUNPATH).*Library runpath: \[\(.*\)\]$/\1/p')
  [ "$runpath" = '$ORIGIN' ] ||
    fail "lib/libvitri_c.so has RUNPATH '$runpath', not \$ORIGIN"
  echo "lib/libvitri_c.so has RUNPATH $runpath"

  # shellcheck disable=SC2086
  "$scripts/no-gmp-inside.sh" "$library" $originals || fail "lib/libvitri_c.so carries GMP inside it"
  echo "ok: $archive carries the GMP in $prefix"
  exit 0
fi

[ "$#" -ge 1 ] || usage
(cd "$(dirname "$1")" && sha256sum -c "$(basename "$1").sha256") || fail "checksum of $1"
extract "$1"
shift

for file in include/vitri.h lib/pkgconfig/vitri-c.pc examples/prepare.c examples/prepare.cpp example.cnf; do
  [ -s "$root/$file" ] || fail "$archive has no $file"
done
for notice in $("$scripts/collect-notices.sh" list gmp); do
  [ -s "$root/notices/$notice" ] || fail "$archive has no notices/$notice"
done

flags=$(PKG_CONFIG_PATH="$root/lib/pkgconfig" pkg-config --cflags --libs vitri-c) ||
  fail "pkg-config cannot read lib/pkgconfig/vitri-c.pc"
echo "pkg-config --cflags --libs vitri-c: $flags"
mkdir "$work/bin"
# shellcheck disable=SC2086
cc -std=c99 "$root/examples/prepare.c" $flags -Wl,-rpath,"$root/lib" -o "$work/bin/prepare-c" ||
  fail "building examples/prepare.c"
# shellcheck disable=SC2086
c++ -std=c++17 "$root/examples/prepare.cpp" $flags -Wl,-rpath,"$root/lib" -o "$work/bin/prepare-cpp" ||
  fail "building examples/prepare.cpp"

# Runs inside a container too, so it uses nothing but sh.
run='
for program in "$1"/bin/prepare-c "$1"/bin/prepare-cpp; do
  out=$(mktemp -d)/bundle
  "$program" "$2/example.cnf" "$out" > /dev/null || { echo "FAIL: $program on example.cnf" >&2; exit 1; }
  for file in reduced.cnf preprocess.json vtree.vtree; do
    [ -s "$out/$file" ] || { echo "FAIL: $program wrote no $file" >&2; exit 1; }
  done
  echo "ok: $program wrote a bundle"
done
'

for program in prepare-c prepare-cpp; do
  deps=$(ldd "$work/bin/$program") || fail "ldd cannot read $program"
  echo "$deps"
  for lib in "$library" "$root"/lib/libgmp*; do
    soname=$(basename "$lib")
    loaded=$(echo "$deps" | awk -v soname="$soname" '$1 == soname && $2 == "=>" { print $3 }')
    [ "$(readlink -f "$loaded")" = "$(readlink -f "$lib")" ] ||
      fail "$program loads $soname from '$loaded', not from the archive"
  done
done
sh -c "$run" sh "$work" "$root"

for image; do
  echo "running the examples in a new $image container"
  docker run --rm --network none -v "$work:$work:ro" "$image" /bin/sh -c "$run" sh "$work" "$root" ||
    fail "the examples in $image"
done
echo "ok: $archive builds and runs"
