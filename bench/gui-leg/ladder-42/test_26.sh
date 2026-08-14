#!/bin/sh
# Feature 26: copy command
export LC_ALL=C
export MINIDB_FILE=./minidb_data
rm -f ./minidb_data
fail() { echo "FAIL(test_26): $1"; exit 1; }
[ -f ./minidb ] || fail "./minidb does not exist"
sh ./minidb set src v || fail set
sh ./minidb copy src dst || fail "copy should exit 0"
[ "$(sh ./minidb get src)" = "v" ] || fail "src should keep the value"
[ "$(sh ./minidb get dst)" = "v" ] || fail "dst should hold the value"
exit 0
