#!/bin/sh
# Feature 24: rename command
export LC_ALL=C
export MINIDB_FILE=./minidb_data
rm -f ./minidb_data
fail() { echo "FAIL(test_24): $1"; exit 1; }
[ -f ./minidb ] || fail "./minidb does not exist"
sh ./minidb set old v || fail set
sh ./minidb rename old new || fail "rename should exit 0"
sh ./minidb get old >/dev/null 2>&1
[ $? -eq 1 ] || fail "old key should be gone"
[ "$(sh ./minidb get new)" = "v" ] || fail "new key should hold the value"
exit 0
