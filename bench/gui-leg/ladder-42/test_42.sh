#!/bin/sh
# Feature 42: readonly mode
export LC_ALL=C
export MINIDB_FILE=./minidb_data
rm -f ./minidb_data
fail() { echo "FAIL(test_42): $1"; exit 1; }
[ -f ./minidb ] || fail "./minidb does not exist"
sh ./minidb set k v || fail set
MINIDB_READONLY=1 sh ./minidb set k w >/dev/null 2>&1
[ $? -eq 4 ] || fail "readonly set should exit 4"
[ "$(sh ./minidb get k)" = "v" ] || fail "value should be unchanged"
[ "$(MINIDB_READONLY=1 sh ./minidb get k)" = "v" ] || fail "readonly get should work"
exit 0
