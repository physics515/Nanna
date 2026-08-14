#!/bin/sh
# Feature 13: values with spaces
export LC_ALL=C
export MINIDB_FILE=./minidb_data
rm -f ./minidb_data
fail() { echo "FAIL(test_13): $1"; exit 1; }
[ -f ./minidb ] || fail "./minidb does not exist"
sh ./minidb set greet "hello world" || fail set
v=$(sh ./minidb get greet)
[ "$v" = "hello world" ] || fail "value with spaces should round-trip, got: $v"
exit 0
