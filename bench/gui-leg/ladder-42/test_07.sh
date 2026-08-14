#!/bin/sh
# Feature 7: del command
export LC_ALL=C
export MINIDB_FILE=./minidb_data
rm -f ./minidb_data
fail() { echo "FAIL(test_07): $1"; exit 1; }
[ -f ./minidb ] || fail "./minidb does not exist"
sh ./minidb set k v || fail set
sh ./minidb del k || fail "del should exit 0"
sh ./minidb get k >/dev/null 2>&1
[ $? -eq 1 ] || fail "get after del should exit 1"
exit 0
