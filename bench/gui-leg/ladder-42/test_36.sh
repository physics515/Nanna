#!/bin/sh
# Feature 36: backup and restore
export LC_ALL=C
export MINIDB_FILE=./minidb_data
rm -f ./minidb_data
fail() { echo "FAIL(test_36): $1"; exit 1; }
[ -f ./minidb ] || fail "./minidb does not exist"
sh ./minidb set k v1 || fail set
sh ./minidb backup b.db || fail "backup should exit 0"
sh ./minidb set k v2 || fail set2
sh ./minidb restore b.db || fail "restore should exit 0"
[ "$(sh ./minidb get k)" = "v1" ] || fail "restore should bring back v1"
rm -f b.db
exit 0
