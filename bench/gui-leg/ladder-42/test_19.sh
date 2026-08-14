#!/bin/sh
# Feature 19: incr creates at 1
export LC_ALL=C
export MINIDB_FILE=./minidb_data
rm -f ./minidb_data
fail() { echo "FAIL(test_19): $1"; exit 1; }
[ -f ./minidb ] || fail "./minidb does not exist"
out=$(sh ./minidb incr m) || fail "incr should exit 0"
[ "$out" = "1" ] || fail "incr missing should print 1, got: $out"
exit 0
