#!/bin/sh
# Feature 33: top command
export LC_ALL=C
export MINIDB_FILE=./minidb_data
rm -f ./minidb_data
fail() { echo "FAIL(test_33): $1"; exit 1; }
[ -f ./minidb ] || fail "./minidb does not exist"
sh ./minidb mset a 5 b 9 c 1 || fail mset
got=$(sh ./minidb top 2)
want=$(printf 'b 9\na 5')
[ "$got" = "$want" ] || fail "top 2 should print b 9 then a 5, got: $got"
exit 0
