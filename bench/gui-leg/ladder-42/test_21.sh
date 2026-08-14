#!/bin/sh
# Feature 21: decr command
export LC_ALL=C
export MINIDB_FILE=./minidb_data
rm -f ./minidb_data
fail() { echo "FAIL(test_21): $1"; exit 1; }
[ -f ./minidb ] || fail "./minidb does not exist"
sh ./minidb set n 5 || fail set
out=$(sh ./minidb decr n) || fail "decr should exit 0"
[ "$out" = "4" ] || fail "decr should print 4, got: $out"
exit 0
