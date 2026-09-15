#!/bin/sh
# Require that an ELF file is a copy of another, by their GNU build IDs.
#
#     same-build-id.sh <file> <original>
#
# Prints the ID when both files have it. Fails when <original> is missing,
# when either file has no build ID, or when the IDs differ. patchelf and
# auditwheel rewrite a library's dynamic section when they bundle it but keep
# its build ID, so a bundled copy still matches the library it was copied from.
# Needs readelf.
set -eu

if [ "$#" -ne 2 ]; then
  sed -n '2,/^set/{/^set/d;s/^# \{0,1\}//;p}' "$0" >&2
  exit 2
fi

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

build_id() {
  readelf -n "$1" | awk '/Build ID:/ { print $3 }'
}

[ -f "$2" ] || fail "$2 does not exist, so $1 is not a copy of it"
id=$(build_id "$1")
[ -n "$id" ] || fail "$1 has no build ID"
[ "$id" = "$(build_id "$2")" ] || fail "$1 is not a copy of $2: their build IDs differ"
echo "$1 has build ID $id, the same as $2"
