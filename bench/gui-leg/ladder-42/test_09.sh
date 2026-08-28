#!/bin/sh
# Feature 9: exists command
export LC_ALL=C
export MINIDB_FILE=./minidb_data
rm -f ./minidb_data
fail() { echo "FAIL(test_09): $1"; exit 1; }
[ -f ./minidb ] || fail "./minidb does not exist"
sh ./minidb set k v || fail set
out=$(sh ./minidb exists k) || fail "exists should exit 0 for present key"
[ -z "$out" ] || fail "exists should print nothing"
sh ./minidb exists other >/dev/null 2>&1
[ $? -eq 1 ] || fail "exists should exit 1 for absent key"
exit 0
