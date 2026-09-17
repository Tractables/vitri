#!/bin/sh
# Check that a binary loads GMP and MPFR rather than carrying them inside it.
#
#     no-gmp-inside.sh <elf> [gmp-library...]
#
# GMP and MPFR are LGPL and are linked dynamically, always, so nothing built
# here may define their symbols. Two checks, and a binary has to pass both:
# no defined symbol is named like one of theirs, and, when the libraries it is
# supposed to load are given, it defines none of the symbols they export.
# Needs nm.
set -eu
export LC_ALL=C

if [ "$#" -lt 1 ]; then
  echo "usage: no-gmp-inside.sh <elf> [gmp-library...]" >&2
  exit 2
fi
elf=$1
shift

# Both tables: a static archive folded into the binary leaves its symbols in
# the symbol table, a shared object exports them in the dynamic one.
defined() {
  { nm --defined-only "$1" 2> /dev/null || true; nm -D --defined-only "$1" 2> /dev/null || true; }
}

if defined "$elf" | grep -Eq ' [A-Za-z] (__gmp[a-z]*_|mpfr_)'; then
  echo "$elf defines GMP or MPFR functions"
  exit 1
fi

[ "$#" -gt 0 ] || exit 0

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
for library in "$@"; do
  nm -D --defined-only "$library"
done |
  awk 'NF == 3 && $2 ~ /^[TDRB]$/ && $3 !~ /^(_init|_fini|_edata|_end|__bss_start)$/ { print $3 }' |
  sort -u > "$work/exported"
if [ ! -s "$work/exported" ]; then
  echo "the given libraries export no symbols"
  exit 1
fi
defined "$elf" | awk 'NF == 3 { print $3 }' | sort -u > "$work/defined"
both=$(comm -12 "$work/exported" "$work/defined")
if [ -n "$both" ]; then
  echo "$elf defines symbols GMP exports, e.g. $(echo "$both" | head -n 3 | tr '\n' ' ')"
  exit 1
fi
echo "$elf defines none of the $(wc -l < "$work/exported") symbols GMP exports"
