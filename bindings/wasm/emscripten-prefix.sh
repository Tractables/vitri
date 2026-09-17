#!/usr/bin/env bash
# Build the prefix `VITRI_EMSCRIPTEN_PREFIX` names: GMP and MPFR compiled for
# Emscripten, and GMP's side modules.
#
#     emscripten-prefix.sh <work-dir> <prefix>
#
# Needs the Emscripten SDK on PATH (emconfigure, emmake, emcc, em++), m4 for
# GMP's configure and curl for the downloads. GMP comes from the tarball
# `.github/scripts/gmp.sh` pins, which is left in <work-dir> for publishing next
# to the module; MPFR from the tarball pinned below. Both are built static, and
# `lib/libgmp.so` and `lib/libgmpxx.so` are then linked from the static
# libraries as side modules. Arjun's CMake requires MPFR and its headers include
# MPFR's, so MPFR is installed; nothing in the module calls it, so it gets no
# side module.
set -euo pipefail

[ "$#" -eq 2 ] || { sed -n '2,/^set/{/^set/d;s/^# \{0,1\}//;p}' "$0" >&2; exit 2; }
work=$1
mkdir -p "$2"
prefix=$(cd "$2" && pwd)
gmp=$(cd "$(dirname "$0")/../.." && pwd)/.github/scripts/gmp.sh

mpfr_version=4.2.1
mpfr_sha256=277807353a6726978996945af13e52829e3abd7a9a5b7fb2793894e18f1fcbb2

# GMP's own tests cannot run under Emscripten. The library is built as one
# generic code path, since the CPU dispatch of --enable-fat has no meaning for
# WebAssembly, and with the exception model the vendored C++ is compiled with.
GMP_CHECK=no CFLAGS="-O3 -fPIC" CXXFLAGS="-O3 -fPIC -fwasm-exceptions" \
  emconfigure "$gmp" build "$work" "$prefix" \
    --host=none --disable-assembly --disable-fat --enable-static --disable-shared

mkdir -p "$work"
work=$(cd "$work" && pwd)
mpfr=mpfr-$mpfr_version
curl -fsSL --retry 3 -o "$work/$mpfr.tar.xz" "https://ftp.gnu.org/gnu/mpfr/$mpfr.tar.xz"
echo "$mpfr_sha256  $work/$mpfr.tar.xz" | sha256sum -c
rm -rf "$work/$mpfr"
tar -xJf "$work/$mpfr.tar.xz" -C "$work"
(
  cd "$work/$mpfr"
  emconfigure ./configure --host=none --prefix="$prefix" --with-gmp="$prefix" \
    --enable-static --disable-shared CC_FOR_BUILD=gcc
  emmake make -j"$(nproc)"
  emmake make install
)

cd "$prefix/lib"
emcc -sSIDE_MODULE=1 -O3 -Wl,--whole-archive libgmp.a -Wl,--no-whole-archive -o libgmp.so
em++ -sSIDE_MODULE=1 -O3 -fwasm-exceptions -Wl,--whole-archive libgmpxx.a -Wl,--no-whole-archive libgmp.so -o libgmpxx.so
