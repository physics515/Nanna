#!/bin/sh
# Feature 15: MINIDB_FILE env
export LC_ALL=C
export MINIDB_FILE=./minidb_data
rm -f ./minidb_data
fail() { echo "FAIL(test_15): $1"; exit 1; }
[ -f ./minidb ] || fail "./minidb does not exist"
MINIDB_FILE=./alt_db sh ./minidb set k v || fail set
[ -f ./alt_db ] || fail "alt_db file should exist"
v=$(MINIDB_FILE=./alt_db sh ./minidb get k)
[ "$v" = "v" ] || fail "get from alt_db should print v"
sh ./minidb get k >/dev/null 2>&1
[ $? -eq 1 ] || fail "default db should not have the key"
rm -f ./alt_db
exit 0
