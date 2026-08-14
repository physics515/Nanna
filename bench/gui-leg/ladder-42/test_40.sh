#!/bin/sh
# Feature 40: nlist command
export LC_ALL=C
export MINIDB_FILE=./minidb_data
rm -f ./minidb_data
fail() { echo "FAIL(test_40): $1"; exit 1; }
[ -f ./minidb ] || fail "./minidb does not exist"
sh ./minidb nset app b 2 || fail n1
sh ./minidb nset app a 1 || fail n2
sh ./minidb nset web c 3 || fail n3
got=$(sh ./minidb nlist app)
want=$(printf 'a\nb')
[ "$got" = "$want" ] || fail "nlist app should print a+b, got: $got"
exit 0
