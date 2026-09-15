#!/usr/bin/env bash
# Build GMP from its upstream source tarball, for release files that bundle it.
#
#     gmp.sh version
#     gmp.sh fetch <work-dir>
#     gmp.sh build <work-dir> <prefix> [configure-argument ...]
#
# `version` prints the GMP version. `fetch` downloads the tarball into
# <work-dir>, or keeps the verified copy already there, checks its sha256 and
# prints its absolute path; that file is the source to publish next to anything
# built from it, and after `build` it is already in <work-dir>. `build` fetches,
# extracts a fresh source tree into <work-dir>, then configures, builds, runs
# GMP's tests and installs into <prefix>: headers in include/, libraries and
# pkg-config files in lib/, and GMP's licence texts in share/licenses/gmp/.
#
# `build` runs configure with --enable-cxx --enable-shared --disable-static
# --enable-fat, then the configure arguments given here, which override those.
# --enable-fat makes the library pick CPU-specific code when it loads instead of
# being tuned to the build machine. Configure takes the compilers and flags from
# CC, CXX, CFLAGS and CXXFLAGS. GMP_CHECK=no skips the tests. MAKEFLAGS, when
# set, replaces the default -j<number of CPUs>.
#
# For Emscripten, set GMP_CHECK=no, CFLAGS="-O3 -fPIC" and
# CXXFLAGS="-O3 -fPIC -fwasm-exceptions", and run
#     emconfigure gmp.sh build <work-dir> <prefix> --host=none --disable-assembly --disable-fat --enable-static --disable-shared
# This installs libgmp.a and libgmpxx.a. Link side modules from them with
#     emcc -sSIDE_MODULE=1 -O3 -Wl,--whole-archive libgmp.a -Wl,--no-whole-archive -o libgmp.so
#     em++ -sSIDE_MODULE=1 -O3 -fwasm-exceptions -Wl,--whole-archive libgmpxx.a -Wl,--no-whole-archive libgmp.so -o libgmpxx.so
set -euo pipefail

version=6.3.0
tarball=gmp-$version.tar.xz
url=https://ftp.gnu.org/gnu/gmp/$tarball
# Checked against the GNU signature file, $url.sig, made with the GMP release
# key 343C 2FF0 FBEE 5EC2 EDBE F399 F359 9FF8 28C6 7298.
sha256=a3c2b80201b89e68616f4ad30bc66aee4927c3ce50e33929ca819d5c43538898

usage() {
  sed -n '2,/^set/{/^set/d;s/^# \{0,1\}//;p}' "$0" >&2
  exit 2
}

check_sha256() {
  if ! echo "$sha256  $1" | sha256sum -c --status; then
    echo "$1 does not have the sha256 of $tarball ($sha256)" >&2
    exit 1
  fi
}

fetch() { # work-dir
  mkdir -p "$1"
  local path
  path=$(cd "$1" && pwd)/$tarball
  if [ ! -e "$path" ]; then
    curl -fsSL --retry 3 -o "$path.part" "$url"
    check_sha256 "$path.part"
    mv "$path.part" "$path"
  fi
  check_sha256 "$path"
  echo "$path"
}

build() { # work-dir prefix [configure-argument ...]
  local work=$1 prefix=$2 source tree
  shift 2
  case ${GMP_CHECK:=yes} in
    yes | no) ;;
    *)
      echo "GMP_CHECK must be yes or no, not '$GMP_CHECK'" >&2
      exit 2
      ;;
  esac
  source=$(fetch "$work")
  mkdir -p "$prefix"
  prefix=$(cd "$prefix" && pwd)
  tree=$(dirname "$source")/gmp-$version
  rm -rf "$tree"
  tar -xJf "$source" -C "$(dirname "$source")"
  cd "$tree"
  ./configure --prefix="$prefix" --enable-cxx --enable-shared --disable-static --enable-fat "$@"
  export MAKEFLAGS=${MAKEFLAGS:--j$(nproc)}
  make
  if [ "$GMP_CHECK" = yes ]; then
    make check
  fi
  make install
  mkdir -p "$prefix/share/licenses/gmp"
  cp COPYING.LESSERv3 COPYINGv3 COPYINGv2 "$prefix/share/licenses/gmp/"
}

case ${1:-} in
  version)
    [ "$#" -eq 1 ] || usage
    echo "$version"
    ;;
  fetch)
    [ "$#" -eq 2 ] || usage
    fetch "$2"
    ;;
  build)
    [ "$#" -ge 3 ] || usage
    shift
    build "$@"
    ;;
  *) usage ;;
esac
