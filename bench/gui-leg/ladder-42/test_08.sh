#!/bin/sh
# Feature 8: del missing key
export LC_ALL=C
export MINIDB_FILE=./minidb_data
rm -f ./minidb_data
fail() { echo "FAIL(test_08): $1"; exit 1; }
[ -f ./minidb ] || fail "./minidb does not exist"
sh ./minidb del nothere >/dev/null 2>&1
[ $? -eq 1 ] || fail "del missing should exit 1"
exit 0
