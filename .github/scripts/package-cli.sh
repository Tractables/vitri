#!/usr/bin/env bash
# Assemble the release archive of the command-line tool on Linux.
#
#     package-cli.sh <binary> <label> <target> <out-dir>
#
# Writes <out-dir>/vitri-<label>-<target>.tar.gz and a .sha256 file beside it.
# <label> is the release tag, or a commit for a build that is not released.
#
# GMP (and MPFR, when the linker keeps it) is LGPL and stays dynamically
# linked, so the archive carries those shared libraries in lib/ and sets the
# binary's RUNPATH to $ORIGIN/../lib. Every other library the binary loads must
# be on the list of base-system libraries below; anything else stops the script,
# so a new dependency is packaged on purpose instead of failing on a user's
# machine.
#
# Needs ldd, readelf, objdump, patchelf and dpkg-query (the licence texts are the
# distribution's copyright files for the libraries it bundles).
set -euo pipefail

if [ "$#" -ne 4 ]; then
  sed -n '2,/^set/{/^set/d;s/^# \{0,1\}//;p}' "$0" >&2
  exit 2
fi
binary=$1 label=$2 target=$3 out=$4

# Copied into lib/.
bundled='^lib(gmp|gmpxx|mpfr)\.so\.[0-9]+$'
# Present on every glibc-based Linux: glibc, the GCC runtime libraries and zlib.
# The archive README states the minimum glibc and libstdc++ versions.
base_system='^(linux-vdso\.so\.1|ld-linux-x86-64\.so\.2|lib(c|m|dl|pthread|rt|gcc_s|stdc\+\+|z)\.so\.[0-9]+)$'

root=$(cd "$(dirname "$0")/../.." && pwd)
name="vitri-$label-$target"
stage="$out/$name"
rm -rf "$stage" "$out/$name.tar.gz" "$out/$name.tar.gz.sha256"
mkdir -p "$stage/bin" "$stage/lib" "$stage/licenses"

cp "$binary" "$stage/bin/vitri"

echo "dynamic dependencies of $binary:"
ldd "$binary"
sources=()
while read -r first arrow path _; do
  soname=${first##*/}
  if [ "$arrow" != "=>" ]; then
    path=$first
  elif [ "$path" = not ]; then
    echo "$soname is not found on this machine" >&2
    exit 1
  fi
  if [[ $soname =~ $bundled ]]; then
    cp -L "$path" "$stage/lib/$soname"
    sources+=("$path")
  elif ! [[ $soname =~ $base_system ]]; then
    echo "$soname is neither bundled nor a base-system library; add it to one list in $0" >&2
    exit 1
  fi
done < <(ldd "$binary")

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

. /etc/os-release
source_list=""
for path in "${sources[@]}"; do
  real=$(readlink -f "$path")
  package=$(dpkg-query -S "$real" | head -n 1 | cut -d: -f1)
  cp "/usr/share/doc/$package/copyright" "$stage/licenses/$package.copyright"
  for text in $(grep -o '/usr/share/common-licenses/[A-Za-z0-9.+-]*[A-Za-z0-9+]' "/usr/share/doc/$package/copyright" | sort -u); do
    # Under the name of the file it resolves to: `GPL` is a link to `GPL-3`.
    cp "$(readlink -f "$text")" "$stage/licenses/"
  done
  read -r source version < <(dpkg-query -W -f '${source:Package} ${source:Version}\n' "$package")
  source_list+="- \`lib/$(basename "$path")\`: package \`$package\`, built from the $NAME source package \`$source\` version \`$version\`"
  if [ "$ID" = ubuntu ]; then
    source_list+=" (<https://launchpad.net/ubuntu/+source/$source/$version>)"
  fi
  source_list+=$'\n'
done

cp "$root/LICENSE" "$root/docs/THIRD-PARTY.md" "$root/docs/example.cnf" "$stage/"

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

## Bundled libraries

\`lib/\` holds the GMP libraries vitri links dynamically; \`bin/vitri\` loads them
from there, so keep \`bin/\` and \`lib/\` side by side. They are licensed under
the LGPL version 3 or later (GMP alternatively under the GPL version 2 or later).
\`licenses/\` holds their copyright files and the licence texts those files name.
Their source code:

$source_list
\`LICENSE\` is vitri's licence; \`THIRD-PARTY.md\` covers the components compiled
into \`bin/vitri\`.
EOF

tar --sort=name --owner=0 --group=0 --numeric-owner -C "$out" -cf - "$name" | gzip -9n > "$out/$name.tar.gz"
(cd "$out" && sha256sum "$name.tar.gz" > "$name.tar.gz.sha256")

echo "RUNPATH of bin/vitri: \$ORIGIN/../lib; needs glibc $glibc, GLIBCXX_$glibcxx"
tar -tzvf "$out/$name.tar.gz"
cat "$out/$name.tar.gz.sha256"
