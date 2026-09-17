#!/usr/bin/env bash
# Assemble the release archive of the command-line tool on Linux.
#
#     package-cli.sh <binary> <label> <target> <gmp-prefix> <out-dir>
#
# Writes <out-dir>/vitri-<label>-<target>.tar.gz and a .sha256 file beside it.
# <label> is the release tag, or a commit for a build that is not released.
# <gmp-prefix> is where gmp.sh installed the GMP the binary was built against.
#
# GMP is LGPL and stays dynamically linked, so the archive carries libgmp and
# libgmpxx from <gmp-prefix> in lib/, with the notices collect-notices.sh
# gathers in notices/, and sets the binary's RUNPATH to $ORIGIN/../lib. Every
# other library the binary loads must be on the list of base-system libraries
# below; anything else stops the script, so a new dependency is packaged on
# purpose instead of failing on a user's machine.
#
# Needs ldd, readelf, objdump, patchelf, cargo and python3.
set -euo pipefail

if [ "$#" -ne 5 ]; then
  sed -n '2,/^set/{/^set/d;s/^# \{0,1\}//;p}' "$0" >&2
  exit 2
fi
binary=$1 label=$2 target=$3 gmp=$4 out=$5

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

name="vitri-$label-$target"
stage="$out/$name"
rm -rf "$stage" "$out/$name.tar.gz" "$out/$name.tar.gz.sha256"
mkdir -p "$stage/bin" "$stage/lib"

cp "$binary" "$stage/bin/vitri"

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

patchelf --set-rpath '$ORIGIN/../lib' "$stage/bin/vitri"
for lib in "$stage"/lib/*; do
  patchelf --set-rpath '$ORIGIN' "$lib"
done
readelf -d "$stage/bin/vitri" | grep -F 'Library runpath: [$ORIGIN/../lib]' ||
  { echo "patchelf did not set the RUNPATH of bin/vitri" >&2; exit 1; }

# The newest symbol versions anything in the archive asks of the base system.
newest() {
  objdump -T "$stage/bin/vitri" "$stage"/lib/* | grep -o "$1[0-9.]*[0-9]" | sort -uV | tail -n 1 | sed "s/^$1//"
}
glibc=$(newest GLIBC_)
glibcxx=$(newest GLIBCXX_)

"$scripts/collect-notices.sh" "$stage/notices" "$gmp"
cp "$root/docs/example.cnf" "$stage/"

. /etc/os-release
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

Linux on x86_64 with glibc $glibc or newer and the GCC C++ runtime
(libstdc++ providing GLIBCXX_$glibcxx or newer). Built on $PRETTY_NAME.

## Licences

\`lib/\` holds the GMP libraries libgmp and libgmpxx, which vitri links
dynamically; \`bin/vitri\` loads them from there, so keep \`bin/\` and \`lib/\` side
by side. GMP is licensed under the LGPL version 3 or later, or the GPL version 2
or later; the texts are \`notices/GMP-COPYING.LESSERv3\`, \`notices/GMP-COPYINGv3\`
and \`notices/GMP-COPYINGv2\`. The source of GMP $gmp_version, which these
libraries are built from, is attached to the vitri release.

\`notices/LICENSE\` is vitri's licence. \`notices/THIRD-PARTY.md\` covers the
components compiled into \`bin/vitri\`, and \`notices/goatd-THIRD-PARTY.md\` those
that come with its goatd dependency.
EOF

tar --sort=name --owner=0 --group=0 --numeric-owner -C "$out" -cf - "$name" | gzip -9n > "$out/$name.tar.gz"
(cd "$out" && sha256sum "$name.tar.gz" > "$name.tar.gz.sha256")

echo "RUNPATH of bin/vitri: \$ORIGIN/../lib; needs glibc $glibc, GLIBCXX_$glibcxx; GMP $gmp_version from $gmp"
tar -tzvf "$out/$name.tar.gz"
cat "$out/$name.tar.gz.sha256"
