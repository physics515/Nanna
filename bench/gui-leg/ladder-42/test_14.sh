#!/bin/sh
# Feature 14: case-sensitive keys
export LC_ALL=C
export MINIDB_FILE=./minidb_data
rm -f ./minidb_data
fail() { echo "FAIL(test_14): $1"; exit 1; }
[ -f ./minidb ] || fail "./minidb does not exist"
sh ./minidb set Key A && sh ./minidb set key B || fail set
[ "$(sh ./minidb get Key)" = "A" ] || fail "Key should be A"
[ "$(sh ./minidb get key)" = "B" ] || fail "key should be B"
exit 0
