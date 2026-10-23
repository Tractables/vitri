#!/usr/bin/env bash
# Assemble a release archive on Linux: the command-line tool, or the C library.
#
#     package.sh cli <vitri> <label> <target> <gmp-prefix> <out-dir>
#     package.sh c <libvitri_c.so> <label> <target> <gmp-prefix> <out-dir>
#
# Writes <out-dir>/vitri-<label>-<target>.tar.gz for the tool, or
# <out-dir>/vitri-c-<label>-<target>.tar.gz for the library, and a .sha256 file
# beside it. <label> is the release tag, or a commit for a build that is not
# released. <gmp-prefix> is where gmp.sh installed the GMP the binary was built
# against.
#
# GMP is LGPL and stays dynamically linked, so the archive carries libgmp and
# libgmpxx from <gmp-prefix> in lib/, with the notices collect-notices.sh
# gathers in notices/. The tool's RUNPATH is $ORIGIN/../lib and the library's
# $ORIGIN. Every other library the binary loads must be on the list of
# base-system libraries below; anything else stops the script, so a new
# dependency is packaged on purpose instead of failing on a user's machine.
#
# The C archive has the shared library only, with the header, the examples and
# a pkg-config file that finds the directory it was unpacked into.
#
# Needs ldd, readelf, objdump, patchelf, cargo and python3.
set -euo pipefail

if [ "$#" -ne 6 ] || { [ "$1" != cli ] && [ "$1" != c ]; }; then
  sed -n '2,/^set/{/^set/d;s/^# \{0,1\}//;p}' "$0" >&2
  exit 2
fi
kind=$1 binary=$2 label=$3 target=$4 gmp=$5 out=$6

# Copied into lib/ from the GMP prefix.
bundled='^lib(gmp|gmpxx)\.so\.[0-9]+$'
# Present on every glibc-based Linux: glibc, the GCC runtime libraries and zlib.
# The archive README states the minimum glibc and libstdc++ versions.
base_system='^(linux-vdso\.so\.1|ld-linux-x86-64\.so\.2|lib(c|m|dl|pthread|rt|gcc_s|stdc\+\+|z)\.so\.[0-9]+)$'

scripts=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$scripts/../.." && pwd)
gmp=$(cd "$gmp" && pwd)

gmp_version=$("$scripts/gmp.sh" version)
built_version=$(awk '/^#define __GNU_MP_VERSION(_MINOR|_PATCHLEVEL)?[ \t]/ { v = v (v == "" ? "" : ".") $3 } END { print v }' "$gmp/include/gmp.h")
if [ "$built_version" != "$gmp_version" ]; then
  echo "$gmp/include/gmp.h is GMP ${built_version:-of unknown version}, but gmp.sh builds $gmp_version" >&2
  exit 1
fi

if [ "$kind" = cli ]; then
  name="vitri-$label-$target"
  installed=bin/vitri
  runpath='$ORIGIN/../lib'
else
  name="vitri-c-$label-$target"
  installed=lib/libvitri_c.so
  runpath='$ORIGIN'
fi
stage="$out/$name"
rm -rf "$stage" "$out/$name.tar.gz" "$out/$name.tar.gz.sha256"
mkdir -p "$stage/lib" "$stage/$(dirname "$installed")"

cp "$binary" "$stage/$installed"

echo "dynamic dependencies of $binary, searching $gmp/lib first:"
deps=$(LD_LIBRARY_PATH="$gmp/lib" ldd "$binary")
echo "$deps"
while read -r first arrow path _; do
  soname=${first##*/}
  if [ "$arrow" != "=>" ]; then
    path=$first
  elif [ "$path" = not ]; then
    echo "$soname is not found on this machine" >&2
    exit 1
  fi
  if [[ $soname =~ $bundled ]]; then
    if [ "$(readlink -f "$path")" != "$(readlink -f "$gmp/lib/$soname")" ]; then
      echo "$soname resolves to $path, not to the copy in $gmp/lib" >&2
      exit 1
    fi
    cp -L "$path" "$stage/lib/$soname"
  elif ! [[ $soname =~ $base_system ]]; then
    echo "$soname is neither bundled nor a base-system library; add it to one list in $0" >&2
    exit 1
  fi
done <<< "$deps"

if ! compgen -G "$stage/lib/libgmp.so.*" > /dev/null; then
  echo "$binary does not load GMP as a shared library" >&2
  exit 1
fi

for lib in "$stage"/lib/*; do
  patchelf --set-rpath '$ORIGIN' "$lib"
done
patchelf --set-rpath "$runpath" "$stage/$installed"
readelf -d "$stage/$installed" | grep -F "Library runpath: [$runpath]" ||
  { echo "patchelf did not set the RUNPATH of $installed" >&2; exit 1; }

# The newest symbol versions anything in the archive asks of the base system.
newest() {
  objdump -T "$stage/$installed" "$stage"/lib/*.so* | grep -o "$1[0-9.]*[0-9]" | sort -uV | tail -n 1 | sed "s/^$1//"
}
glibc=$(newest GLIBC_)
glibcxx=$(newest GLIBCXX_)

"$scripts/collect-notices.sh" "$stage/notices" "$gmp"
cp "$root/docs/example.cnf" "$stage/"

. /etc/os-release
requirements="Linux on x86_64 with glibc $glibc or newer and the GCC C++ runtime
(libstdc++ providing GLIBCXX_$glibcxx or newer). Built on $PRETTY_NAME."
gmp_texts="GMP is licensed under the LGPL version 3 or later, or the GPL version 2
or later; the texts are \`notices/GMP-COPYING.LESSERv3\`, \`notices/GMP-COPYINGv3\`
and \`notices/GMP-COPYINGv2\`. The source of GMP $gmp_version, which these
libraries are built from, is attached to the vitri release."

if [ "$kind" = cli ]; then
  cat > "$stage/README.md" <<EOF
# vitri $label for $target

\`bin/vitri\` is the vitri command-line tool. From this directory:

\`\`\`sh
bin/vitri example.cnf --out-dir bundle/
\`\`\`

\`bin/vitri --help\` lists the options. The documentation for this version is at
<https://github.com/Tractables/vitri/tree/$label>: \`README.md\` for usage,
\`docs/getting-started.md\` for passing the bundle to a compiler, and
\`docs/bundle.md\` for the files it contains.

## Requirements

$requirements

## Licences

\`lib/\` holds the GMP libraries libgmp and libgmpxx, which vitri links
dynamically; \`bin/vitri\` loads them from there, so keep \`bin/\` and \`lib/\` side
by side. $gmp_texts

\`notices/LICENSE\` is vitri's licence. \`notices/THIRD-PARTY.md\` covers the
components compiled into \`bin/vitri\`, and \`notices/goatd-THIRD-PARTY.md\` those
that come with its goatd dependency.
EOF
else
  mkdir -p "$stage/include" "$stage/examples" "$stage/lib/pkgconfig"
  cp "$root/bindings/c/include/vitri.h" "$stage/include/"
  cp "$root/bindings/c/examples/prepare.c" "$root/bindings/c/examples/prepare.cpp" "$stage/examples/"
  version=$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/bindings/c/Cargo.toml" | head -n 1)
  # Libs.private and its comment are for the static library, which is not here.
  sed -e 's|@PREFIX@|${pcfiledir}/../..|' -e "s|@VERSION@|$version|" -e '/^#/d' -e '/^Libs\.private:/d' \
    "$root/bindings/c/vitri-c.pc.in" > "$stage/lib/pkgconfig/vitri-c.pc"
  cat > "$stage/README.md" <<EOF
# vitri C library $label for $target

\`include/vitri.h\` declares the C API and documents every function, and
\`lib/libvitri_c.so\` implements it. From this directory:

\`\`\`sh
cc -Iinclude examples/prepare.c -Llib -lvitri_c -Wl,-rpath,"\$PWD/lib" -o prepare
./prepare example.cnf bundle/
\`\`\`

\`examples/prepare.cpp\` is the same program in C++. \`lib/pkgconfig/vitri-c.pc\`
gives pkg-config the compile and link flags for wherever this directory is
unpacked; the program still needs \`-Wl,-rpath\` or \`LD_LIBRARY_PATH\` to find
\`lib/\` at run time. The documentation for this version is at
<https://github.com/Tractables/vitri/tree/$label/bindings/c>. The static
library is not included; build it from source if you need it.

## Requirements

$requirements

## Licences

\`lib/\` holds the GMP libraries libgmp and libgmpxx, which libvitri_c.so links
dynamically and loads from beside it, so keep them in one directory. $gmp_texts

\`notices/LICENSE\` is vitri's licence. \`notices/THIRD-PARTY.md\` covers the
components compiled into \`lib/libvitri_c.so\`, and \`notices/goatd-THIRD-PARTY.md\`
those that come with its goatd dependency.
EOF
fi

tar --sort=name --owner=0 --group=0 --numeric-owner -C "$out" -cf - "$name" | gzip -9n > "$out/$name.tar.gz"
(cd "$out" && sha256sum "$name.tar.gz" > "$name.tar.gz.sha256")

echo "RUNPATH of $installed: $runpath; needs glibc $glibc, GLIBCXX_$glibcxx; GMP $gmp_version from $gmp"
tar -tzvf "$out/$name.tar.gz"
cat "$out/$name.tar.gz.sha256"
