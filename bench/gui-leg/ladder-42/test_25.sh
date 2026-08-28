#!/bin/sh
# Feature 25: rename missing
export LC_ALL=C
export MINIDB_FILE=./minidb_data
rm -f ./minidb_data
fail() { echo "FAIL(test_25): $1"; exit 1; }
[ -f ./minidb ] || fail "./minidb does not exist"
sh ./minidb rename ghost dst >/dev/null 2>&1
[ $? -eq 1 ] || fail "rename missing should exit 1"
exit 0
