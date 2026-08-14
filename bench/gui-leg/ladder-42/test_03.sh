#!/bin/sh
# Feature 3: set and get
export LC_ALL=C
export MINIDB_FILE=./minidb_data
rm -f ./minidb_data
fail() { echo "FAIL(test_03): $1"; exit 1; }
[ -f ./minidb ] || fail "./minidb does not exist"
sh ./minidb set name alice || fail "set should exit 0"
v=$(sh ./minidb get name) || fail "get should exit 0"
[ "$v" = "alice" ] || fail "get should print alice, got: $v"
exit 0
