#!/bin/sh
# Feature 16: append command
export LC_ALL=C
export MINIDB_FILE=./minidb_data
rm -f ./minidb_data
fail() { echo "FAIL(test_16): $1"; exit 1; }
[ -f ./minidb ] || fail "./minidb does not exist"
sh ./minidb set k ab || fail set
sh ./minidb append k cd || fail "append should exit 0"
[ "$(sh ./minidb get k)" = "abcd" ] || fail "append should concatenate"
exit 0
