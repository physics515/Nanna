#!/bin/sh
# Feature 18: incr command
export LC_ALL=C
export MINIDB_FILE=./minidb_data
rm -f ./minidb_data
fail() { echo "FAIL(test_18): $1"; exit 1; }
[ -f ./minidb ] || fail "./minidb does not exist"
sh ./minidb set n 5 || fail set
out=$(sh ./minidb incr n) || fail "incr should exit 0"
[ "$out" = "6" ] || fail "incr should print 6, got: $out"
[ "$(sh ./minidb get n)" = "6" ] || fail "stored value should be 6"
exit 0
