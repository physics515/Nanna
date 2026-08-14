#!/bin/sh
# Feature 17: append creates missing
export LC_ALL=C
export MINIDB_FILE=./minidb_data
rm -f ./minidb_data
fail() { echo "FAIL(test_17): $1"; exit 1; }
[ -f ./minidb ] || fail "./minidb does not exist"
sh ./minidb append fresh xy || fail "append should exit 0"
[ "$(sh ./minidb get fresh)" = "xy" ] || fail "append should create the key"
exit 0
