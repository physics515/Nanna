#!/bin/sh
# Feature 27: search command
export LC_ALL=C
export MINIDB_FILE=./minidb_data
rm -f ./minidb_data
fail() { echo "FAIL(test_27): $1"; exit 1; }
[ -f ./minidb ] || fail "./minidb does not exist"
sh ./minidb mset apple 1 apricot 2 banana 3 || fail mset
got=$(sh ./minidb search ap)
want=$(printf 'apple\napricot')
[ "$got" = "$want" ] || fail "search ap should print apple+apricot, got: $got"
exit 0
