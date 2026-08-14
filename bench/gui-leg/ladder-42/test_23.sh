#!/bin/sh
# Feature 23: mget command
export LC_ALL=C
export MINIDB_FILE=./minidb_data
rm -f ./minidb_data
fail() { echo "FAIL(test_23): $1"; exit 1; }
[ -f ./minidb ] || fail "./minidb does not exist"
sh ./minidb set a 1 && sh ./minidb set b 2 || fail set
got=$(sh ./minidb mget a b)
want=$(printf '1\n2')
[ "$got" = "$want" ] || fail "mget should print values in order, got: $got"
exit 0
